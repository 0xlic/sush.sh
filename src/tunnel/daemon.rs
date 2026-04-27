use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

use crate::config::store;
use crate::tunnel::ipc::{self, ForwardState, ForwardStatus, IpcRequest, IpcResponse, MAX_RETRIES};

fn daemon_sock_path() -> Result<std::path::PathBuf> {
    Ok(store::config_dir().join("daemon.sock"))
}

fn daemon_pid_path() -> Result<std::path::PathBuf> {
    Ok(store::config_dir().join("daemon.pid"))
}

fn ensure_runtime_parent(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

struct RuntimeFiles {
    sock_path: std::path::PathBuf,
    pid_path: std::path::PathBuf,
}

impl RuntimeFiles {
    fn new(sock_path: std::path::PathBuf, pid_path: std::path::PathBuf) -> Self {
        Self {
            sock_path,
            pid_path,
        }
    }
}

impl Drop for RuntimeFiles {
    fn drop(&mut self) {
        std::fs::remove_file(&self.sock_path).ok();
        std::fs::remove_file(&self.pid_path).ok();
    }
}

#[cfg(unix)]
async fn ping_existing_daemon(sock: &std::path::Path) -> bool {
    match UnixStream::connect(sock).await {
        Ok(mut s) => {
            let req = ipc::encode_request(&IpcRequest::Status).unwrap_or_default();
            s.write_all(&req).await.is_ok()
        }
        Err(_) => false,
    }
}

#[derive(Debug)]
struct RuleState {
    state: ForwardState,
    retry_count: u32,
    error: Option<String>,
    cancel: Option<tokio_util::sync::CancellationToken>,
    stopped: bool,
}

#[allow(dead_code)]
impl RuleState {
    fn new() -> Self {
        Self {
            state: ForwardState::Stopped,
            retry_count: 0,
            error: None,
            cancel: None,
            stopped: false,
        }
    }

    fn on_connect_success(&mut self, cancel: tokio_util::sync::CancellationToken) {
        self.state = ForwardState::Running;
        self.retry_count = 0;
        self.error = None;
        self.cancel = Some(cancel);
        self.stopped = false;
    }

    fn on_disconnect(&mut self) {
        self.cancel = None;
        self.retry_count += 1;
        if self.retry_count >= MAX_RETRIES {
            self.state = ForwardState::Error;
            self.error = Some("max retries exceeded".into());
        } else {
            self.state = ForwardState::Reconnecting;
        }
    }

    fn on_fatal_error(&mut self, msg: String) {
        self.state = ForwardState::Error;
        self.error = Some(msg);
        self.cancel = None;
        self.stopped = false;
    }

    fn reset_for_manual_start(&mut self) {
        self.state = ForwardState::Connecting;
        self.retry_count = 0;
        self.error = None;
        self.stopped = false;
    }

    fn stop(&mut self) {
        if let Some(token) = self.cancel.take() {
            token.cancel();
        }
        self.state = ForwardState::Stopped;
        self.retry_count = 0;
        self.error = None;
        self.stopped = true;
    }
}

type SharedState = Arc<Mutex<HashMap<String, RuleState>>>;

#[cfg(unix)]
pub async fn run_daemon() -> Result<()> {
    let sock_path = daemon_sock_path()?;
    ensure_runtime_parent(&sock_path)?;

    if sock_path.exists() && ping_existing_daemon(&sock_path).await {
        eprintln!("sush daemon is already running");
        return Ok(());
    }
    if sock_path.exists() {
        std::fs::remove_file(&sock_path).ok();
    }

    let config_store = store::load_store(&store::config_path())?;
    let state: SharedState = Arc::new(Mutex::new(HashMap::new()));

    {
        let mut locked = state.lock().await;
        for host in &config_store.hosts {
            for rule in &host.forwards {
                locked.insert(rule.id.clone(), RuleState::new());
            }
        }
    }

    for host in &config_store.hosts {
        for rule in &host.forwards {
            if rule.auto_start {
                let _ = prepare_rule_for_start(&state, &rule.id).await;
                let state = Arc::clone(&state);
                let host_clone = host.clone();
                let rule_clone = rule.clone();
                let all_hosts = config_store.hosts.clone();
                tokio::spawn(async move {
                    start_rule_task(&host_clone, &rule_clone, &all_hosts, state).await;
                });
            }
        }
    }

    let listener = UnixListener::bind(&sock_path)?;
    let pid_path = daemon_pid_path()?;
    ensure_runtime_parent(&pid_path)?;
    std::fs::write(&pid_path, std::process::id().to_string())?;
    let _runtime_files = RuntimeFiles::new(sock_path.clone(), pid_path);
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    eprintln!("sush daemon listening on {sock_path:?}");

    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                break;
            }
            _ = sigint.recv() => {
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _)) => {
                        let state = Arc::clone(&state);
                        tokio::spawn(handle_connection(stream, state));
                    }
                    Err(e) => eprintln!("daemon accept error: {e}"),
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn handle_connection(stream: UnixStream, state: SharedState) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let response = match serde_json::from_str::<IpcRequest>(&line) {
            Ok(req) => handle_request(req, &state).await,
            Err(e) => IpcResponse::Error {
                message: e.to_string(),
            },
        };
        if let Ok(bytes) = ipc::encode_response(&response)
            && writer.write_all(&bytes).await.is_err()
        {
            break;
        }
    }
}

#[cfg(unix)]
async fn handle_request(req: IpcRequest, state: &SharedState) -> IpcResponse {
    let hosts = match store::load_store(&store::config_path()) {
        Ok(store_data) => store_data.hosts,
        Err(error) => {
            return IpcResponse::Error {
                message: format!("failed to load config: {error}"),
            };
        }
    };
    sync_state_with_hosts(state, &hosts).await;
    handle_request_with_hosts(req, state, &hosts).await
}

async fn handle_request_with_hosts(
    req: IpcRequest,
    state: &SharedState,
    hosts: &[crate::config::host::Host],
) -> IpcResponse {
    match req {
        IpcRequest::Status => {
            let locked = state.lock().await;
            let statuses: Vec<ForwardStatus> = hosts
                .iter()
                .flat_map(|h| {
                    h.forwards.iter().map(|r| {
                        let rs = locked.get(&r.id);
                        ForwardStatus {
                            id: r.id.clone(),
                            host_id: h.id.clone(),
                            state: rs.map(|s| s.state.clone()).unwrap_or(ForwardState::Stopped),
                            retry_count: rs.map(|s| s.retry_count).unwrap_or(0),
                            error: rs.and_then(|s| s.error.clone()),
                        }
                    })
                })
                .collect();
            IpcResponse::Status(statuses)
        }
        IpcRequest::Start { forward_id } => {
            let (host_opt, rule_opt) = find_rule(hosts, &forward_id);
            match (host_opt, rule_opt) {
                (Some(host), Some(rule)) => {
                    if !prepare_rule_for_start(state, &forward_id).await {
                        let locked = state.lock().await;
                        if locked
                            .get(&forward_id)
                            .map(|entry| entry.state.is_active())
                            .unwrap_or(false)
                        {
                            return IpcResponse::Ok;
                        }
                    }
                    let state = Arc::clone(state);
                    let host = host.clone();
                    let rule = rule.clone();
                    let all_hosts = hosts.to_vec();
                    tokio::spawn(async move {
                        start_rule_task(&host, &rule, &all_hosts, state).await;
                    });
                    IpcResponse::Ok
                }
                _ => IpcResponse::Error {
                    message: format!("forward rule {forward_id} not found"),
                },
            }
        }
        IpcRequest::Stop { forward_id } => {
            let mut locked = state.lock().await;
            if let Some(rs) = locked.get_mut(&forward_id) {
                rs.stop();
            }
            IpcResponse::Ok
        }
        IpcRequest::StopAll => {
            let mut locked = state.lock().await;
            for rs in locked.values_mut() {
                rs.stop();
            }
            IpcResponse::Ok
        }
    }
}

async fn sync_state_with_hosts(state: &SharedState, hosts: &[crate::config::host::Host]) {
    let active_ids: std::collections::HashSet<String> = hosts
        .iter()
        .flat_map(|host| host.forwards.iter().map(|rule| rule.id.clone()))
        .collect();

    let mut locked = state.lock().await;
    for host in hosts {
        for rule in &host.forwards {
            locked.entry(rule.id.clone()).or_insert_with(RuleState::new);
        }
    }

    let stale_ids: Vec<String> = locked
        .keys()
        .filter(|rule_id| !active_ids.contains(*rule_id))
        .cloned()
        .collect();
    for stale_id in stale_ids {
        if let Some(mut rule_state) = locked.remove(&stale_id) {
            rule_state.stop();
        }
    }
}

async fn prepare_rule_for_start(state: &SharedState, rule_id: &str) -> bool {
    let mut locked = state.lock().await;
    let entry = locked
        .entry(rule_id.to_string())
        .or_insert_with(RuleState::new);
    if entry.state.is_active() {
        return false;
    }
    entry.reset_for_manual_start();
    true
}

async fn sleep_or_stop(state: &SharedState, rule_id: &str, wait_secs: u64) -> bool {
    let sleep = tokio::time::sleep(std::time::Duration::from_secs(wait_secs));
    tokio::pin!(sleep);

    loop {
        if state
            .lock()
            .await
            .get(rule_id)
            .map(|rs| rs.stopped)
            .unwrap_or(true)
        {
            return false;
        }

        tokio::select! {
            _ = &mut sleep => return true,
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
        }
    }
}

fn find_rule<'a>(
    hosts: &'a [crate::config::host::Host],
    forward_id: &str,
) -> (
    Option<&'a crate::config::host::Host>,
    Option<&'a crate::config::host::ForwardRule>,
) {
    for host in hosts {
        for rule in &host.forwards {
            if rule.id == forward_id {
                return (Some(host), Some(rule));
            }
        }
    }
    (None, None)
}

// Placeholder — filled in Task 6 (wire start_rule_task)
#[cfg(unix)]
async fn start_rule_task(
    host: &crate::config::host::Host,
    rule: &crate::config::host::ForwardRule,
    all_hosts: &[crate::config::host::Host],
    state: SharedState,
) {
    use crate::config::host::ForwardKind;
    use crate::ssh::auth;
    use crate::ssh::proxy_jump;
    use crate::ssh::session::{ActiveSession, ClientHandler};
    use crate::tunnel::forward;

    let rule_id = rule.id.clone();
    let backoff_secs = [1u64, 2, 4, 8, 16, 30];

    loop {
        let mut forwarded_rx = None;
        let handler = if matches!(rule.kind, ForwardKind::Remote) {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            forwarded_rx = Some(rx);
            ClientHandler::with_forwarded_tcpip(tx)
        } else {
            ClientHandler::default()
        };

        let handle_result = if let Some(ref jump_alias) = host.proxy_jump {
            match all_hosts
                .iter()
                .find(|candidate| candidate.alias == *jump_alias)
            {
                Some(bastion) => {
                    proxy_jump::connect_via_proxy_jump_with_handler(bastion, host, handler).await
                }
                None => Err(anyhow::anyhow!("proxy jump host '{jump_alias}' not found")),
            }
        } else {
            match ActiveSession::connect_with_handler(&host.hostname, host.port, handler).await {
                Ok(mut session) => match auth::authenticate(&mut session.handle, host).await {
                    Ok(()) => Ok(session.handle),
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            }
        };

        let handle = match handle_result {
            Ok(handle) => handle,
            Err(error) => {
                if is_fatal_error(&error) {
                    let mut locked = state.lock().await;
                    if let Some(rs) = locked.get_mut(&rule_id) {
                        rs.on_fatal_error(error.to_string());
                    }
                    return;
                }
                let maybe_wait = {
                    let mut locked = state.lock().await;
                    locked.get_mut(&rule_id).map(|rs| {
                        rs.on_disconnect();
                        let retry = rs.retry_count as usize;
                        (
                            rs.state.clone(),
                            backoff_secs
                                .get(retry.saturating_sub(1))
                                .copied()
                                .unwrap_or(30),
                        )
                    })
                };
                let Some((next_state, wait)) = maybe_wait else {
                    return;
                };
                if next_state == ForwardState::Error {
                    let mut locked = state.lock().await;
                    if let Some(rs) = locked.get_mut(&rule_id) {
                        rs.error = Some(error.to_string());
                    }
                    return;
                }
                eprintln!("forward {rule_id}: connect failed ({error}), retry in {wait}s");
                if !sleep_or_stop(&state, &rule_id, wait).await {
                    return;
                }
                continue;
            }
        };

        let cancel = tokio_util::sync::CancellationToken::new();
        {
            let mut locked = state.lock().await;
            if let Some(rs) = locked.get_mut(&rule_id) {
                if rs.state == ForwardState::Stopped {
                    return;
                }
                rs.on_connect_success(cancel.clone());
            }
        }

        let forward_result = match &rule.kind {
            ForwardKind::Local => match (rule.remote_host.clone(), rule.remote_port) {
                (Some(remote_host), Some(remote_port)) => {
                    forward::run_local_forward(
                        handle,
                        rule.local_port,
                        remote_host,
                        remote_port,
                        cancel,
                    )
                    .await
                }
                _ => Err(anyhow::anyhow!(
                    "local forward '{}' is missing remote host or remote port",
                    rule.name
                )),
            },
            ForwardKind::Dynamic => {
                forward::run_dynamic_forward(handle, rule.local_port, cancel).await
            }
            ForwardKind::Remote => match (rule.remote_host.clone(), rule.remote_port, forwarded_rx)
            {
                (Some(remote_host), Some(remote_port), Some(forwarded_rx)) => {
                    forward::run_remote_forward_with_receiver(
                        handle,
                        forwarded_rx,
                        remote_host,
                        remote_port,
                        rule.local_port,
                        cancel,
                    )
                    .await
                }
                _ => Err(anyhow::anyhow!(
                    "remote forward '{}' is missing remote port",
                    rule.name
                )),
            },
        };

        if let Err(ref error) = forward_result {
            eprintln!("forward {rule_id} error: {error}");
        }
        if let Err(ref error) = forward_result
            && is_fatal_error(error)
        {
            let mut locked = state.lock().await;
            if let Some(rs) = locked.get_mut(&rule_id) {
                rs.on_fatal_error(error.to_string());
            }
            return;
        }

        {
            let locked = state.lock().await;
            if locked
                .get(&rule_id)
                .map(|rs| rs.state == ForwardState::Stopped)
                .unwrap_or(false)
            {
                return;
            }
        }

        let maybe_wait = {
            let mut locked = state.lock().await;
            locked.get_mut(&rule_id).map(|rs| {
                rs.on_disconnect();
                if let Some(err) = &forward_result.as_ref().err() {
                    rs.error = Some(err.to_string());
                }
                let retry = rs.retry_count as usize;
                (
                    rs.state.clone(),
                    backoff_secs
                        .get(retry.saturating_sub(1))
                        .copied()
                        .unwrap_or(30),
                )
            })
        };
        let Some((next_state, wait)) = maybe_wait else {
            return;
        };
        if next_state == ForwardState::Error {
            return;
        }

        eprintln!("forward {rule_id}: disconnected, retry in {wait}s");
        if !sleep_or_stop(&state, &rule_id, wait).await {
            return;
        }
    }
}

fn is_fatal_error(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("all authentication methods failed")
        || text.contains("authentication failed")
        || text.contains("proxy jump host")
        || text.contains("missing remote")
        || text.contains("already in use")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::host::{ForwardKind, ForwardRule, Host, HostSource};
    use tempfile::TempDir;

    fn sample_host_with_forwards() -> Host {
        Host {
            id: "host-1".into(),
            alias: "host-1".into(),
            hostname: "127.0.0.1".into(),
            port: 22,
            user: "tester".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: vec![],
            description: String::new(),
            source: HostSource::Manual,
            forwards: vec![
                ForwardRule {
                    id: "fwd-1".into(),
                    name: "web".into(),
                    kind: ForwardKind::Local,
                    local_port: 8080,
                    remote_host: Some("localhost".into()),
                    remote_port: Some(80),
                    auto_start: false,
                },
                ForwardRule {
                    id: "fwd-2".into(),
                    name: "socks".into(),
                    kind: ForwardKind::Dynamic,
                    local_port: 1080,
                    remote_host: None,
                    remote_port: None,
                    auto_start: false,
                },
            ],
        }
    }

    #[test]
    fn state_machine_happy_path() {
        let mut s = RuleState::new();
        assert_eq!(s.state, ForwardState::Stopped);
        s.reset_for_manual_start();
        assert_eq!(s.state, ForwardState::Connecting);
        let token = tokio_util::sync::CancellationToken::new();
        s.on_connect_success(token);
        assert_eq!(s.state, ForwardState::Running);
        assert_eq!(s.retry_count, 0);
    }

    #[test]
    fn state_machine_reconnect_then_error() {
        let mut s = RuleState::new();
        s.reset_for_manual_start();
        let token = tokio_util::sync::CancellationToken::new();
        s.on_connect_success(token);
        for i in 1..=MAX_RETRIES {
            s.on_disconnect();
            assert_eq!(s.retry_count, i);
            if i < MAX_RETRIES {
                assert_eq!(s.state, ForwardState::Reconnecting);
            }
        }
        assert_eq!(s.state, ForwardState::Error);
    }

    #[test]
    fn stop_clears_state() {
        let mut s = RuleState::new();
        s.reset_for_manual_start();
        let token = tokio_util::sync::CancellationToken::new();
        s.on_connect_success(token);
        s.stop();
        assert_eq!(s.state, ForwardState::Stopped);
        assert_eq!(s.retry_count, 0);
    }

    #[test]
    fn ensure_runtime_parent_creates_missing_directory() {
        let temp = TempDir::new().unwrap();
        let sock_path = temp.path().join("nested").join("daemon.sock");

        assert!(!sock_path.parent().unwrap().exists());

        ensure_runtime_parent(&sock_path).unwrap();

        assert!(sock_path.parent().unwrap().is_dir());
    }

    #[test]
    fn runtime_files_drop_removes_pid_and_socket() {
        let temp = TempDir::new().unwrap();
        let sock_path = temp.path().join("daemon.sock");
        let pid_path = temp.path().join("daemon.pid");
        std::fs::write(&sock_path, "").unwrap();
        std::fs::write(&pid_path, "").unwrap();

        {
            let _runtime_files = RuntimeFiles::new(sock_path.clone(), pid_path.clone());
        }

        assert!(!sock_path.exists());
        assert!(!pid_path.exists());
    }

    #[tokio::test]
    async fn handle_request_status_reports_rule_state() {
        let hosts = vec![sample_host_with_forwards()];
        let state: SharedState = Arc::new(Mutex::new(HashMap::from([(
            "fwd-1".into(),
            RuleState {
                state: ForwardState::Running,
                retry_count: 0,
                error: None,
                cancel: None,
                stopped: false,
            },
        )])));

        let response = handle_request_with_hosts(IpcRequest::Status, &state, &hosts).await;

        let IpcResponse::Status(statuses) = response else {
            panic!("expected status response");
        };

        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].id, "fwd-1");
        assert_eq!(statuses[0].state, ForwardState::Running);
        assert_eq!(statuses[1].id, "fwd-2");
        assert_eq!(statuses[1].state, ForwardState::Stopped);
    }

    #[tokio::test]
    async fn handle_request_start_sets_known_rule_to_connecting() {
        let hosts = vec![sample_host_with_forwards()];
        let state: SharedState = Arc::new(Mutex::new(HashMap::new()));

        let response = handle_request_with_hosts(
            IpcRequest::Start {
                forward_id: "fwd-1".into(),
            },
            &state,
            &hosts,
        )
        .await;

        assert!(matches!(response, IpcResponse::Ok));

        let locked = state.lock().await;
        let rule = locked.get("fwd-1").unwrap();
        assert_eq!(rule.state, ForwardState::Connecting);
        assert_eq!(rule.retry_count, 0);
        assert!(rule.error.is_none());
    }

    #[tokio::test]
    async fn prepare_rule_for_start_sets_known_rule_to_connecting() {
        let state: SharedState = Arc::new(Mutex::new(HashMap::from([(
            "fwd-1".into(),
            RuleState::new(),
        )])));

        assert!(prepare_rule_for_start(&state, "fwd-1").await);

        let locked = state.lock().await;
        let rule = locked.get("fwd-1").unwrap();
        assert_eq!(rule.state, ForwardState::Connecting);
        assert_eq!(rule.retry_count, 0);
        assert!(!rule.stopped);
    }

    #[tokio::test]
    async fn handle_request_start_returns_error_for_unknown_rule() {
        let hosts = vec![sample_host_with_forwards()];
        let state: SharedState = Arc::new(Mutex::new(HashMap::new()));

        let response = handle_request_with_hosts(
            IpcRequest::Start {
                forward_id: "missing".into(),
            },
            &state,
            &hosts,
        )
        .await;

        let IpcResponse::Error { message } = response else {
            panic!("expected error response");
        };

        assert!(message.contains("missing"));
    }

    #[tokio::test]
    async fn handle_request_stop_only_resets_target_rule() {
        let hosts = vec![sample_host_with_forwards()];
        let state: SharedState = Arc::new(Mutex::new(HashMap::from([
            (
                "fwd-1".into(),
                RuleState {
                    state: ForwardState::Running,
                    retry_count: 2,
                    error: Some("boom".into()),
                    cancel: None,
                    stopped: false,
                },
            ),
            (
                "fwd-2".into(),
                RuleState {
                    state: ForwardState::Running,
                    retry_count: 1,
                    error: None,
                    cancel: None,
                    stopped: false,
                },
            ),
        ])));

        let response = handle_request_with_hosts(
            IpcRequest::Stop {
                forward_id: "fwd-1".into(),
            },
            &state,
            &hosts,
        )
        .await;

        assert!(matches!(response, IpcResponse::Ok));

        let locked = state.lock().await;
        assert_eq!(locked.get("fwd-1").unwrap().state, ForwardState::Stopped);
        assert_eq!(locked.get("fwd-1").unwrap().retry_count, 0);
        assert_eq!(locked.get("fwd-2").unwrap().state, ForwardState::Running);
        assert_eq!(locked.get("fwd-2").unwrap().retry_count, 1);
    }

    #[tokio::test]
    async fn handle_request_stop_all_resets_every_rule() {
        let hosts = vec![sample_host_with_forwards()];
        let state: SharedState = Arc::new(Mutex::new(HashMap::from([
            (
                "fwd-1".into(),
                RuleState {
                    state: ForwardState::Running,
                    retry_count: 2,
                    error: Some("boom".into()),
                    cancel: None,
                    stopped: false,
                },
            ),
            (
                "fwd-2".into(),
                RuleState {
                    state: ForwardState::Reconnecting,
                    retry_count: 1,
                    error: None,
                    cancel: None,
                    stopped: false,
                },
            ),
        ])));

        let response = handle_request_with_hosts(IpcRequest::StopAll, &state, &hosts).await;
        assert!(matches!(response, IpcResponse::Ok));

        let locked = state.lock().await;
        for rule in locked.values() {
            assert_eq!(rule.state, ForwardState::Stopped);
            assert_eq!(rule.retry_count, 0);
            assert!(rule.error.is_none());
        }
    }

    #[tokio::test]
    async fn sync_state_with_hosts_adds_new_rules_and_removes_deleted_rules() {
        let state: SharedState = Arc::new(Mutex::new(HashMap::from([(
            "deleted-rule".into(),
            RuleState {
                state: ForwardState::Running,
                retry_count: 1,
                error: None,
                cancel: None,
                stopped: false,
            },
        )])));
        let hosts = vec![sample_host_with_forwards()];

        sync_state_with_hosts(&state, &hosts).await;

        let locked = state.lock().await;
        assert!(locked.contains_key("fwd-1"));
        assert!(locked.contains_key("fwd-2"));
        assert!(!locked.contains_key("deleted-rule"));
    }

    #[tokio::test]
    async fn sleep_or_stop_returns_false_when_rule_is_stopped() {
        let state: SharedState = Arc::new(Mutex::new(HashMap::from([(
            "fwd-1".into(),
            RuleState {
                state: ForwardState::Reconnecting,
                retry_count: 1,
                error: None,
                cancel: None,
                stopped: false,
            },
        )])));

        let state_for_stop = Arc::clone(&state);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let mut locked = state_for_stop.lock().await;
            locked.get_mut("fwd-1").unwrap().stop();
        });

        assert!(!sleep_or_stop(&state, "fwd-1", 1).await);
    }

    #[test]
    fn fatal_error_detection_matches_expected_cases() {
        assert!(is_fatal_error(&anyhow::anyhow!(
            "all authentication methods failed for prod"
        )));
        assert!(is_fatal_error(&anyhow::anyhow!(
            "proxy jump: authentication failed for bastion"
        )));
        assert!(is_fatal_error(&anyhow::anyhow!(
            "proxy jump host 'bastion' not found"
        )));
        assert!(is_fatal_error(&anyhow::anyhow!(
            "port 8080 already in use: bind failed"
        )));
        assert!(!is_fatal_error(&anyhow::anyhow!(
            "connection reset by peer"
        )));
    }
}
