use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

use crate::config::store;
use crate::tunnel::ipc::{self, ForwardState, ForwardStatus, IpcRequest, IpcResponse, MAX_RETRIES};

fn daemon_sock_path() -> Result<std::path::PathBuf> {
    Ok(store::config_dir().join("daemon.sock"))
}

fn daemon_pid_path() -> Result<std::path::PathBuf> {
    Ok(store::config_dir().join("daemon.pid"))
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
}

#[allow(dead_code)]
impl RuleState {
    fn new() -> Self {
        Self {
            state: ForwardState::Stopped,
            retry_count: 0,
            error: None,
            cancel: None,
        }
    }

    fn on_connect_success(&mut self, cancel: tokio_util::sync::CancellationToken) {
        self.state = ForwardState::Running;
        self.retry_count = 0;
        self.error = None;
        self.cancel = Some(cancel);
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
    }

    fn reset_for_manual_start(&mut self) {
        self.state = ForwardState::Connecting;
        self.retry_count = 0;
        self.error = None;
    }

    fn stop(&mut self) {
        if let Some(token) = self.cancel.take() {
            token.cancel();
        }
        self.state = ForwardState::Stopped;
        self.retry_count = 0;
        self.error = None;
    }
}

type SharedState = Arc<Mutex<HashMap<String, RuleState>>>;

#[cfg(unix)]
pub async fn run_daemon() -> Result<()> {
    let sock_path = daemon_sock_path()?;

    if sock_path.exists() && ping_existing_daemon(&sock_path).await {
        eprintln!("sush daemon is already running");
        return Ok(());
    }
    if sock_path.exists() {
        std::fs::remove_file(&sock_path).ok();
    }

    let pid_path = daemon_pid_path()?;
    std::fs::write(&pid_path, std::process::id().to_string())?;

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
    eprintln!("sush daemon listening on {sock_path:?}");

    let hosts = config_store.hosts.clone();
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let state = Arc::clone(&state);
                let hosts = hosts.clone();
                tokio::spawn(handle_connection(stream, state, hosts));
            }
            Err(e) => eprintln!("daemon accept error: {e}"),
        }
    }
}

#[cfg(unix)]
async fn handle_connection(
    stream: UnixStream,
    state: SharedState,
    hosts: Vec<crate::config::host::Host>,
) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let response = match serde_json::from_str::<IpcRequest>(&line) {
            Ok(req) => handle_request(req, &state, &hosts).await,
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
async fn handle_request(
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
                    {
                        let mut locked = state.lock().await;
                        let entry = locked
                            .entry(forward_id.clone())
                            .or_insert_with(RuleState::new);
                        if entry.state.is_active() {
                            return IpcResponse::Ok;
                        }
                        entry.reset_for_manual_start();
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
    eprintln!("start_rule_task: {} (not yet implemented)", rule.id);
    let _ = (host, rule, all_hosts, &state);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
