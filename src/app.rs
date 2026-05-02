use std::collections::{VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::io::{Stdout, stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;
use russh::ChannelMsg;

use crate::config::history::ConnectionHistory;
use crate::config::host::Host;
use crate::config::secrets::{
    SecretError, SecretKey, SecretKind, SecretStore, SystemSecretBackend,
};
use crate::config::store;
use crate::sftp::client::{SftpClient, list_local};
use crate::sftp::transfer::{
    RecursiveAggregateProgress, RecursiveTransferPlan, TransferProgress, TransferState,
    build_local_recursive_plan, build_remote_recursive_plan, download, upload,
};
use crate::sftp::{PaneSide, SftpPaneState};
use crate::ssh::auth;
use crate::ssh::proxy_jump;
use crate::ssh::session::{ActiveSession, ClientHandler, try_key_auth};
use crate::ssh::terminal::TerminalEmulator;
use crate::tui::event::{AppEvent, EventBus};
use crate::tui::views::edit_view::{self, EditDraft, EditField};
use crate::tui::views::folder_view::{
    self, FolderFocus, FolderViewState, JumpState, SearchState, hosts_in_path, jump_candidates,
    level_1_paths, parent_path,
};
use crate::tui::views::import_view::{self, ImportViewState};
use crate::tui::views::{main_view, password_dialog::PasswordDialog, sftp_view, ssh_view};
use crate::tui::widgets::confirm_dialog::{ChoiceDialog, ConfirmDialog};
use crate::tui::widgets::status_bar::TransferBadge;
use crate::utils::open;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainFocus {
    HostList,
    Directory,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Main,
    Ssh,
    Sftp,
    Edit,
    ImportSshConfig,
    #[allow(dead_code)]
    FolderView,
    ForwardingManager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardingFocus {
    HostList,
    RuleList,
}

pub struct ForwardingViewState {
    pub focus: ForwardingFocus,
    pub host_list_state: ListState,
    pub rule_list_state: ListState,
    pub host_indices: Vec<usize>,
    pub statuses: Vec<crate::tunnel::ipc::ForwardStatus>,
}

impl ForwardingViewState {
    pub fn new(hosts: &[Host]) -> Self {
        let host_indices: Vec<usize> = hosts.iter().enumerate().map(|(index, _)| index).collect();
        let mut host_list_state = ListState::default();
        if !host_indices.is_empty() {
            host_list_state.select(Some(0));
        }
        Self {
            focus: ForwardingFocus::HostList,
            host_list_state,
            rule_list_state: ListState::default(),
            host_indices,
            statuses: vec![],
        }
    }

    pub fn selected_host_idx(&self) -> Option<usize> {
        self.host_list_state
            .selected()
            .and_then(|i| self.host_indices.get(i))
            .copied()
    }
}

struct PwdDialog {
    dialog: PasswordDialog,
    result: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingConnectionKind {
    Ssh,
    Sftp,
}

#[derive(Debug, Clone)]
enum ConnectionRoute {
    Direct(Box<Host>),
    ViaProxyJump {
        bastion: Box<Host>,
        target: Box<Host>,
    },
}

struct PendingConnection {
    kind: PendingConnectionKind,
    host: Host,
    route: ConnectionRoute,
    status: String,
    rx: tokio::sync::oneshot::Receiver<Result<ActiveSession>>,
}

enum LoopEvent {
    App(AppEvent),
    Ssh(Option<ChannelMsg>),
}

/// Transfer direction determines which pane is refreshed after completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDir {
    Download, // remote -> local, refresh local when done
    Upload,   // local -> remote, refresh remote when done
}

pub struct ActiveTransfer {
    #[allow(dead_code)]
    pub verb: &'static str,
    pub dir: TransferDir,
    pub progress: TransferProgress,
    pub rx: tokio::sync::mpsc::Receiver<TransferProgress>,
    pub cancel: tokio_util::sync::CancellationToken,
    pub done_at: Option<Instant>,
    pub needs_refresh: bool, // true until the completion-triggered refresh runs
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedTransfer {
    SingleFile {
        dir: TransferDir,
        display_name: String,
        local_path: PathBuf,
        remote_path: String,
        total_bytes: u64,
    },
    Recursive {
        display_name: String,
        plan: RecursiveTransferPlan,
    },
}

#[derive(Debug, Clone)]
struct ActiveRecursiveTransfer {
    plan: RecursiveTransferPlan,
    progress: RecursiveAggregateProgress,
    directories_prepared: bool,
    next_file_index: usize,
}

impl ActiveRecursiveTransfer {
    fn new(plan: RecursiveTransferPlan) -> Self {
        let total_files = plan.files.len();
        Self {
            plan,
            progress: RecursiveAggregateProgress::new(total_files),
            directories_prepared: false,
            next_file_index: 0,
        }
    }

    fn mark_file_pending(&mut self, file_name: &str, total_bytes: u64) {
        self.progress.start_file(file_name.into(), total_bytes);
    }

    fn update_current_file_bytes(&mut self, transferred_bytes: u64) {
        self.progress.update_bytes(transferred_bytes);
    }

    fn finish_pending_file(&mut self) {
        self.progress.finish_file();
        self.next_file_index += 1;
    }

    #[allow(dead_code)]
    fn skip_pending_file(&mut self) {
        self.finish_pending_file();
    }

    fn pending_file(&self) -> Option<&crate::sftp::transfer::PlannedFile> {
        self.plan.files.get(self.next_file_index)
    }

    fn current_file_position(&self) -> usize {
        (self.next_file_index + 1).min(self.progress.total_files.max(1))
    }

    fn is_complete(&self) -> bool {
        self.next_file_index >= self.plan.files.len()
    }

    fn sync_transfer_progress(&self, progress: &mut TransferProgress) {
        progress.current_file_index = self.current_file_position();
        progress.total_files = self.progress.total_files.max(1);
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum RemoteEditSyncState {
    Opening,
    Watching,
    Uploading,
    UploadFailed,
    Closed,
}

#[allow(dead_code)]
struct RemoteEditSession {
    remote_path: String,
    local_path: PathBuf,
    workspace: tempfile::TempDir,
    last_uploaded_fingerprint: String,
    last_seen_fingerprint: String,
    sync_state: RemoteEditSyncState,
    last_error: Option<String>,
    watch_state: RemoteEditWatchState,
    watcher: Option<RecommendedWatcher>,
    watch_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
}

impl RemoteEditSession {
    fn with_runtime(
        remote_path: String,
        local_path: PathBuf,
        workspace: tempfile::TempDir,
        initial_fingerprint: String,
        watcher: Option<RecommendedWatcher>,
        watch_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    ) -> Self {
        Self {
            remote_path,
            local_path,
            workspace,
            last_uploaded_fingerprint: initial_fingerprint.clone(),
            last_seen_fingerprint: initial_fingerprint.clone(),
            sync_state: RemoteEditSyncState::Opening,
            last_error: None,
            watch_state: RemoteEditWatchState::new(initial_fingerprint.clone()),
            watcher,
            watch_rx,
        }
    }

    #[cfg(test)]
    fn for_test(remote_path: String, local_path: PathBuf, initial_fingerprint: String) -> Self {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            remote_path,
            local_path,
            workspace: tempfile::tempdir().unwrap(),
            last_uploaded_fingerprint: initial_fingerprint.clone(),
            last_seen_fingerprint: initial_fingerprint.clone(),
            sync_state: RemoteEditSyncState::Opening,
            last_error: None,
            watch_state: RemoteEditWatchState::new(initial_fingerprint),
            watcher: None,
            watch_rx: rx,
        }
    }

    #[allow(dead_code)]
    fn mark_watching(&mut self) {
        self.sync_state = RemoteEditSyncState::Watching;
        self.last_error = None;
    }

    #[allow(dead_code)]
    fn mark_uploading(&mut self) {
        self.sync_state = RemoteEditSyncState::Uploading;
    }

    fn mark_upload_failed(&mut self, error: String) {
        self.sync_state = RemoteEditSyncState::UploadFailed;
        self.last_error = Some(error);
    }

    #[allow(dead_code)]
    fn mark_uploaded(&mut self, fingerprint: String) {
        self.last_uploaded_fingerprint = fingerprint.clone();
        self.last_seen_fingerprint = fingerprint;
        self.sync_state = RemoteEditSyncState::Watching;
        self.last_error = None;
    }

    fn should_upload(&mut self, fingerprint: &str) -> bool {
        self.last_seen_fingerprint = fingerprint.to_string();
        self.watch_state.should_upload(fingerprint) && self.last_uploaded_fingerprint != fingerprint
    }

    fn display_name(&self) -> String {
        Path::new(&self.remote_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.remote_path.clone())
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct RemoteEditWatchState {
    last_observed_fingerprint: String,
}

impl RemoteEditWatchState {
    fn new(initial_fingerprint: String) -> Self {
        Self {
            last_observed_fingerprint: initial_fingerprint,
        }
    }

    fn should_upload(&mut self, fingerprint: &str) -> bool {
        if self.last_observed_fingerprint == fingerprint {
            return false;
        }
        self.last_observed_fingerprint = fingerprint.to_string();
        true
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileConflictChoice {
    Skip,
    Overwrite,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct ConflictResolutionState {
    default_file_conflict: Option<FileConflictChoice>,
}

#[allow(dead_code)]
impl ConflictResolutionState {
    fn apply_choice(&mut self, choice: FileConflictChoice, apply_to_remaining: bool) {
        self.default_file_conflict = apply_to_remaining.then_some(choice);
    }
}

#[derive(Debug, Clone)]
struct RecursiveConflictPrompt {
    file_name: String,
    apply_to_remaining: bool,
}

impl RecursiveConflictPrompt {
    fn new(file_name: String) -> Self {
        Self {
            file_name,
            apply_to_remaining: false,
        }
    }

    fn toggle_apply_to_remaining(&mut self) {
        self.apply_to_remaining = !self.apply_to_remaining;
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureChoice {
    Retry,
    Skip,
    Cancel,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct RecursiveFailureState {
    file_name: String,
    should_retry_current_file: bool,
    should_skip_current_file: bool,
    should_cancel_task: bool,
}

#[allow(dead_code)]
impl RecursiveFailureState {
    fn for_file(file_name: &str) -> Self {
        Self {
            file_name: file_name.into(),
            should_retry_current_file: false,
            should_skip_current_file: false,
            should_cancel_task: false,
        }
    }

    fn apply(&mut self, choice: FailureChoice) {
        self.should_retry_current_file = matches!(choice, FailureChoice::Retry);
        self.should_skip_current_file = matches!(choice, FailureChoice::Skip);
        self.should_cancel_task = matches!(choice, FailureChoice::Cancel);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SftpDeleteConfirmState {
    side: PaneSide,
    selected_count: usize,
}

pub struct App {
    pub mode: AppMode,
    pub hosts: Vec<Host>,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub should_quit: bool,
    pub main_focus: MainFocus,
    main_focus_before_search: MainFocus,
    pub show_folder_sidebar: bool,
    pub list_state: ListState,
    pub trigger_connect: bool,
    pub trigger_sftp: bool,
    pub trigger_pane_enter: bool,
    pub trigger_download: bool,
    pub trigger_upload: bool,
    pub trigger_sftp_delete: bool,
    pub trigger_remote_edit: bool,
    pub trigger_refresh_local: bool,
    pub trigger_refresh_remote: bool,
    pub trigger_ssh_resume: bool,
    pub trigger_ssh_to_sftp: bool,
    pub active_session: Option<ActiveSession>,
    pub sftp_client: Option<SftpClient>,
    pub sftp_pane: Option<SftpPaneState>,
    pub current_host_alias: Option<String>,
    pub active_transfer: Option<ActiveTransfer>,
    pub queued_transfers: VecDeque<QueuedTransfer>,
    queue_completed_count: usize,
    queue_total_count: usize,
    active_recursive_transfer: Option<ActiveRecursiveTransfer>,
    conflict_resolution: ConflictResolutionState,
    recursive_conflict_prompt: Option<RecursiveConflictPrompt>,
    recursive_failure_prompt: Option<RecursiveFailureState>,
    #[allow(dead_code)]
    remote_edit_session: Option<RemoteEditSession>,
    pub last_ctrl_c: Option<Instant>,
    pub ssh_last_size: Option<(u16, u16)>,
    pub terminal_emulator: Option<TerminalEmulator>,
    pub edit_draft: Option<EditDraft>,
    pub import_state: Option<ImportViewState>,
    pub confirm_delete: bool,
    sftp_delete_confirm: Option<SftpDeleteConfirmState>,
    pub import_prompted: bool,
    pub metadata: store::Metadata,
    pub connection_history: ConnectionHistory,
    pub hover_host_id: Option<String>,
    pub hover_since: Option<Instant>,
    pub probe_rx: Option<tokio::sync::oneshot::Receiver<bool>>,
    pub probe_result: Option<bool>,
    pub status_msg: Option<(String, Instant)>,
    pending_connection: Option<PendingConnection>,
    pub show_import_prompt: bool,
    pub folder_view_state: Option<FolderViewState>,
    pub folder_host_indices: Vec<usize>,
    pub forwarding_state: Option<ForwardingViewState>,
    pub forward_edit: Option<crate::tui::views::forward_edit::ForwardEditState>,
    secret_store: SecretStore,
    pwd_dialog: Option<PwdDialog>,
    #[cfg(test)]
    test_config_dir: Option<tempfile::TempDir>,
}

impl App {
    pub fn new() -> Result<Self> {
        let store = store::load_store(&store::config_path())?;
        let hosts = store.hosts;
        let metadata = store.metadata;
        let import_prompted = metadata.import_prompted;
        let history_path = store::config_dir().join("history.toml");
        let connection_history = ConnectionHistory::load(history_path);

        let filtered_indices: Vec<usize> = (0..hosts.len()).collect();
        let mut list_state = ListState::default();
        if !filtered_indices.is_empty() {
            list_state.select(Some(0));
        }

        Ok(Self {
            mode: AppMode::Main,
            hosts,
            search_query: String::new(),
            filtered_indices,
            should_quit: false,
            main_focus: MainFocus::HostList,
            main_focus_before_search: MainFocus::HostList,
            show_folder_sidebar: false,
            list_state,
            trigger_connect: false,
            trigger_sftp: false,
            trigger_pane_enter: false,
            trigger_download: false,
            trigger_upload: false,
            trigger_sftp_delete: false,
            trigger_remote_edit: false,
            trigger_refresh_local: false,
            trigger_refresh_remote: false,
            trigger_ssh_resume: false,
            trigger_ssh_to_sftp: false,
            active_session: None,
            sftp_client: None,
            sftp_pane: None,
            current_host_alias: None,
            active_transfer: None,
            queued_transfers: VecDeque::new(),
            queue_completed_count: 0,
            queue_total_count: 0,
            active_recursive_transfer: None,
            conflict_resolution: ConflictResolutionState::default(),
            recursive_conflict_prompt: None,
            recursive_failure_prompt: None,
            remote_edit_session: None,
            last_ctrl_c: None,
            ssh_last_size: None,
            terminal_emulator: None,
            edit_draft: None,
            import_state: None,
            confirm_delete: false,
            sftp_delete_confirm: None,
            import_prompted,
            metadata,
            connection_history,
            hover_host_id: None,
            hover_since: None,
            probe_rx: None,
            probe_result: None,
            status_msg: None,
            pending_connection: None,
            show_import_prompt: false,
            folder_view_state: None,
            folder_host_indices: vec![],
            forwarding_state: None,
            forward_edit: None,
            secret_store: SecretStore::new(Box::new(SystemSecretBackend::new())),
            pwd_dialog: None,
            #[cfg(test)]
            test_config_dir: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal()?;
        self.check_first_launch_import();
        let result = self.event_loop(&mut terminal).await;
        restore_terminal(&mut terminal)?;
        result
    }

    fn check_first_launch_import(&mut self) {
        if self.import_prompted || !self.hosts.is_empty() {
            return;
        }
        let has_ssh_hosts = crate::config::ssh_config::import_ssh_config()
            .map(|(h, _)| !h.is_empty())
            .unwrap_or(false);
        if has_ssh_hosts {
            self.show_import_prompt = true;
        }
    }

    pub fn global_transfer_badge(&self) -> Option<TransferBadge> {
        let direction = if let Some(active) = &self.active_transfer {
            active.dir
        } else if let Some(recursive) = &self.active_recursive_transfer {
            recursive.plan.dir
        } else {
            return None;
        };
        let direction_symbol = match direction {
            TransferDir::Upload => "\u{2191}",
            TransferDir::Download => "\u{2193}",
        };
        let percent = self
            .active_transfer
            .as_ref()
            .map(|active| {
                active
                    .progress
                    .transferred_bytes
                    .saturating_mul(100)
                    .checked_div(active.progress.total_bytes)
                    .unwrap_or(0)
                    .min(100) as u8
            })
            .unwrap_or(0);

        let total_count = self.queue_total_count.max(
            self.queue_completed_count
                + self.queued_transfers.len()
                + usize::from(
                    self.active_transfer.is_some() || self.active_recursive_transfer.is_some(),
                ),
        );
        let current_index = (self.queue_completed_count + 1).min(total_count.max(1));

        Some(TransferBadge {
            direction_symbol,
            current_index,
            total_count: total_count.max(1),
            percent,
        })
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let mut bus = EventBus::new();

        while !self.should_quit {
            self.poll_pending_connection(terminal, &mut bus).await?;

            // Trigger SSH connection.
            if self.trigger_connect {
                self.trigger_connect = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let host = self.hosts[idx].clone();
                    if self.pending_connection.is_none()
                        && let Err(e) =
                            self.start_pending_connection(PendingConnectionKind::Ssh, host)
                    {
                        self.connection_history.record(
                            &self.hosts[self
                                .filtered_indices
                                .get(self.list_state.selected().unwrap_or(0))
                                .copied()
                                .unwrap_or(idx)]
                            .alias,
                        );
                        self.set_status(format!("Connection failed: {e}"));
                    }
                }
            }

            // Trigger SFTP connection.
            if self.trigger_sftp {
                self.trigger_sftp = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let host = self.hosts[idx].clone();
                    if self.pending_connection.is_none()
                        && let Err(e) =
                            self.start_pending_connection(PendingConnectionKind::Sftp, host)
                    {
                        self.connection_history.record(
                            &self.hosts[self
                                .filtered_indices
                                .get(self.list_state.selected().unwrap_or(0))
                                .copied()
                                .unwrap_or(idx)]
                            .alias,
                        );
                        self.set_status(format!("SFTP failed: {e}"));
                    }
                }
            }

            if self.trigger_ssh_to_sftp {
                self.trigger_ssh_to_sftp = false;
                if let Err(e) = self.switch_ssh_to_sftp(terminal).await {
                    eprintln!("[SFTP error: {e}]");
                    self.leave_ssh_mode(terminal).await?;
                }
            }

            // Trigger SFTP directory navigation.
            if self.trigger_pane_enter {
                self.trigger_pane_enter = false;
                if let Err(e) = self.pane_enter().await {
                    eprintln!("[Navigation error: {e}]");
                }
            }

            // Trigger download.
            if self.trigger_download {
                self.trigger_download = false;
                if let Err(e) = self.start_download().await {
                    eprintln!("[Download error: {e}]");
                }
            }

            // Trigger upload.
            if self.trigger_upload {
                self.trigger_upload = false;
                if let Err(e) = self.start_upload().await {
                    eprintln!("[Upload error: {e}]");
                }
            }

            if self.trigger_sftp_delete {
                self.trigger_sftp_delete = false;
                if let Err(e) = self.start_sftp_delete().await {
                    eprintln!("[Delete error: {e}]");
                }
            }

            if self.trigger_remote_edit {
                self.trigger_remote_edit = false;
                if let Err(e) = self.start_remote_edit().await {
                    self.set_status(format!("Remote edit failed: {e}"));
                }
            }

            // Refresh the local directory after a transfer completes.
            if self.trigger_refresh_local {
                self.trigger_refresh_local = false;
                if let Some(pane) = &mut self.sftp_pane
                    && let Ok(entries) = list_local(&pane.local_path)
                {
                    pane.local_entries = entries;
                }
            }

            // Refresh the remote directory after a transfer completes.
            if self.trigger_refresh_remote {
                self.trigger_refresh_remote = false;
                if let Some(pane) = &self.sftp_pane {
                    let path = pane.remote_path.clone();
                    if let Some(client) = &self.sftp_client
                        && let Ok(entries) = client.list_dir(&path).await
                        && let Some(pane) = &mut self.sftp_pane
                    {
                        pane.remote_entries = entries;
                    }
                }
            }

            // Trigger SSH resume when switching back from SFTP.
            if self.trigger_ssh_resume {
                self.trigger_ssh_resume = false;
                if let Err(e) = self.resume_ssh_from_sftp().await {
                    eprintln!("[SSH error: {e}]");
                    self.leave_ssh_mode(terminal).await?;
                }
            }

            self.poll_transfer_queue().await?;

            self.render(terminal)?;

            let next = if self.mode == AppMode::Ssh {
                if let Some(session) = self.active_session.as_mut().filter(|s| s.has_pty()) {
                    tokio::select! {
                        ev = bus.next() => ev.map(LoopEvent::App),
                        msg = session.wait_channel_msg() => Some(LoopEvent::Ssh(msg?)),
                    }
                } else {
                    bus.next().await.map(LoopEvent::App)
                }
            } else {
                bus.next().await.map(LoopEvent::App)
            };

            match next {
                Some(LoopEvent::App(ev)) => self.handle_event(ev, terminal).await?,
                Some(LoopEvent::Ssh(msg)) => self.handle_ssh_channel_msg(terminal, msg).await?,
                None => break,
            }
        }
        bus.shutdown();
        Ok(())
    }

    async fn handle_event(
        &mut self,
        ev: AppEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        match ev {
            AppEvent::Input(data) => self.handle_input(data).await,
            AppEvent::Tick => self.handle_tick(terminal).await,
        }
    }

    async fn handle_tick(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        if self.mode == AppMode::Ssh {
            self.sync_ssh_size().await?;
            if self
                .active_session
                .as_ref()
                .is_some_and(|session| !session.has_pty())
            {
                self.leave_ssh_mode(terminal).await?;
            }
        }
        self.poll_remote_edit().await?;
        // Clear status message after 4 seconds.
        if let Some((_, since)) = &self.status_msg
            && since.elapsed() >= Duration::from_secs(4)
        {
            self.status_msg = None;
        }
        // Connectivity probe — only in Main mode.
        if self.mode == AppMode::ForwardingManager
            && let Some(state) = &mut self.forwarding_state
        {
            state.statuses = crate::tunnel::client::daemon_status().await;
        }
        if self.mode == AppMode::Main {
            // Collect completed probe result.
            if let Some(rx) = &mut self.probe_rx
                && let Ok(result) = rx.try_recv()
            {
                self.probe_result = Some(result);
                self.probe_rx = None;
            }
            // Start probe after 1s hover, if none is running or completed.
            if self.probe_rx.is_none()
                && self.probe_result.is_none()
                && self
                    .hover_since
                    .is_some_and(|s| s.elapsed() >= std::time::Duration::from_secs(1))
                && let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
            {
                let hostname = self.hosts[idx].hostname.clone();
                let port = self.hosts[idx].port;
                let (tx, rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    let ok = tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        tokio::net::TcpStream::connect((hostname.as_str(), port)),
                    )
                    .await
                    .map(|r| r.is_ok())
                    .unwrap_or(false);
                    let _ = tx.send(ok);
                });
                self.probe_rx = Some(rx);
            }
        }
        Ok(())
    }

    async fn handle_input(&mut self, data: Vec<u8>) -> Result<()> {
        if self.pwd_dialog.is_some() {
            for key in decode_tui_keys(&data) {
                self.handle_pwd_key(key);
            }
            return Ok(());
        }

        match self.mode {
            AppMode::Main | AppMode::Sftp | AppMode::ForwardingManager => {
                for key in decode_tui_keys(&data) {
                    self.handle_key(key);
                }
                Ok(())
            }
            AppMode::Ssh => self.handle_ssh_input(data).await,
            AppMode::Edit => {
                for key in decode_tui_keys(&data) {
                    self.handle_edit_input(key);
                }
                Ok(())
            }
            AppMode::ImportSshConfig => {
                for key in decode_tui_keys(&data) {
                    self.handle_import_input(key);
                }
                Ok(())
            }
            AppMode::FolderView => {
                for key in decode_tui_keys(&data) {
                    self.handle_folder_input(key);
                }
                Ok(())
            }
        }
    }

    fn handle_key(&mut self, k: KeyEvent) {
        if self.pwd_dialog.is_some() {
            self.handle_pwd_key(k);
            return;
        }
        if self.confirm_delete {
            self.handle_confirm_delete(k);
            return;
        }
        if self.sftp_delete_confirm.is_some() {
            self.handle_sftp_delete_confirm(k);
            return;
        }
        if self.show_import_prompt {
            self.handle_import_prompt_key(k);
            return;
        }
        if self.recursive_conflict_prompt.is_some() {
            self.handle_recursive_conflict_key(k);
            return;
        }
        if self.recursive_failure_prompt.is_some() {
            self.handle_recursive_failure_key(k);
            return;
        }
        match self.mode {
            AppMode::Main => self.handle_main_key(k),
            AppMode::Sftp => self.handle_sftp_key(k),
            AppMode::ForwardingManager => self.handle_forwarding_key(k),
            AppMode::Ssh | AppMode::Edit | AppMode::ImportSshConfig | AppMode::FolderView => {}
        }
    }

    fn handle_pwd_key(&mut self, k: KeyEvent) {
        let Some(pwd) = self.pwd_dialog.as_mut() else {
            return;
        };
        match k.code {
            KeyCode::Enter => {
                let input = pwd.dialog.input.clone();
                pwd.result = Some(Some(input));
            }
            KeyCode::Esc => {
                pwd.result = Some(None);
            }
            KeyCode::Backspace => {
                pwd.dialog.input.pop();
            }
            KeyCode::Char(c) => {
                pwd.dialog.input.push(c);
            }
            _ => {}
        }
    }

    pub fn folder_search_prefix(&self) -> Option<&str> {
        self.show_folder_sidebar
            .then(|| {
                self.folder_view_state
                    .as_ref()
                    .map(FolderViewState::selected_path)
            })
            .flatten()
    }

    fn handle_main_key(&mut self, k: KeyEvent) {
        if self.main_focus == MainFocus::Directory
            && self
                .folder_view_state
                .as_ref()
                .and_then(|fv| fv.jump.as_ref())
                .is_some()
        {
            self.handle_folder_jump_key(k);
            return;
        }

        match self.main_focus {
            MainFocus::HostList => self.handle_main_key_hostlist(k),
            MainFocus::Directory => self.handle_main_key_directory(k),
            MainFocus::Search => self.handle_main_key_search(k),
        }
    }

    fn handle_main_key_hostlist(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Tab, _) if self.show_folder_sidebar => {
                self.main_focus = MainFocus::Directory;
            }
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.main_focus_before_search = MainFocus::HostList;
                self.main_focus = MainFocus::Search;
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                self.trigger_connect = true;
            }
            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                self.trigger_sftp = true;
            }
            (KeyCode::Up, _) => self.select_previous(),
            (KeyCode::Down, _) => self.select_next(),
            (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.should_quit = true;
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            (KeyCode::Char('n'), KeyModifiers::NONE) => {
                self.edit_draft = Some(EditDraft::new_host());
                self.mode = AppMode::Edit;
            }
            (KeyCode::Char('e'), KeyModifiers::NONE) => {
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    self.edit_draft = Some(EditDraft::from_host(&self.hosts[idx]));
                    self.mode = AppMode::Edit;
                }
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) if !self.filtered_indices.is_empty() => {
                self.confirm_delete = true;
            }
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                self.open_import_view();
            }
            (KeyCode::Char('f'), KeyModifiers::NONE) => {
                self.toggle_folder_sidebar();
            }
            (KeyCode::Char('p'), KeyModifiers::NONE) => {
                self.forwarding_state = Some(ForwardingViewState::new(&self.hosts));
                self.forward_edit = None;
                self.mode = AppMode::ForwardingManager;
            }
            _ => {}
        }
    }

    fn handle_main_key_directory(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Tab, _) => {
                self.main_focus = MainFocus::HostList;
            }
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.main_focus_before_search = MainFocus::Directory;
                self.main_focus = MainFocus::Search;
            }
            (KeyCode::Char('f'), KeyModifiers::NONE) => {
                self.toggle_folder_sidebar();
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) => {
                if let Some(fv) = self.folder_view_state.as_mut() {
                    let candidates = jump_candidates("", &fv.tree);
                    fv.jump = Some(JumpState {
                        input: String::new(),
                        candidates,
                        sel: 0,
                    });
                }
            }
            (KeyCode::Enter, _) | (KeyCode::Right, _) => {
                self.directory_enter();
            }
            (KeyCode::Backspace, _) | (KeyCode::Left, _) => {
                self.directory_back();
            }
            (KeyCode::Up, _) => {
                self.directory_select_previous();
            }
            (KeyCode::Down, _) => {
                self.directory_select_next();
            }
            _ => {}
        }
    }

    fn handle_main_key_search(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Tab, _) | (KeyCode::Esc, _) | (KeyCode::Enter, _) => {
                self.main_focus = self.main_focus_before_search;
            }
            (KeyCode::Up, _) => self.select_previous(),
            (KeyCode::Down, _) => self.select_next(),
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.search_query.clear();
                self.apply_search();
            }
            (KeyCode::Backspace, _) => {
                self.search_query.pop();
                self.apply_search();
            }
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                self.search_query.push(c);
                self.apply_search();
            }
            _ => {}
        }
    }

    fn handle_sftp_key(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                let now = Instant::now();
                if let Some(last) = self.last_ctrl_c
                    && last.elapsed().as_millis() < 500
                {
                    if self.remote_edit_session.is_some() {
                        self.set_status("Finish remote editing before leaving SFTP".into());
                    } else {
                        self.exit_sftp();
                    }
                    return;
                }
                self.last_ctrl_c = Some(now);
                if let Some(tr) = &self.active_transfer {
                    tr.cancel.cancel();
                }
            }
            (KeyCode::Char('q'), KeyModifiers::NONE) => {
                if self.remote_edit_session.is_some() {
                    self.set_status("Finish remote editing before leaving SFTP".into());
                } else {
                    self.exit_sftp();
                }
            }
            (KeyCode::Esc, KeyModifiers::NONE) => {
                if let Some(pane) = &mut self.sftp_pane {
                    pane.clear_active_selection();
                }
            }
            _ if is_switch_key(k) => self.trigger_ssh_resume = true,
            (KeyCode::Char(' '), KeyModifiers::NONE) => {
                if let Some(pane) = &mut self.sftp_pane {
                    pane.toggle_active_selection();
                }
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) => self.trigger_download = true,
            (KeyCode::Char('u'), KeyModifiers::NONE) => self.trigger_upload = true,
            (KeyCode::Char('D'), KeyModifiers::NONE) => self.trigger_sftp_delete_confirm(),
            (KeyCode::Char('e'), KeyModifiers::NONE) => self.trigger_remote_edit(),
            // D delete and r rename are not implemented yet.
            (KeyCode::Tab, _) => self.toggle_pane(),
            (KeyCode::Up, _) => self.pane_select_prev(),
            (KeyCode::Down, _) => self.pane_select_next(),
            (KeyCode::Enter, _) => self.trigger_pane_enter = true,
            _ => {}
        }
    }

    fn trigger_remote_edit(&mut self) {
        let Some(pane) = &self.sftp_pane else {
            return;
        };
        if pane.side != PaneSide::Remote {
            self.set_status("Remote edit only works from the remote pane".into());
            return;
        }
        let idx = pane.selected_index();
        let Some(entry) = pane.remote_entries.get(idx) else {
            return;
        };
        if entry.is_dir {
            self.set_status("Remote edit only works for files".into());
            return;
        }
        self.trigger_remote_edit = true;
    }

    fn trigger_sftp_delete_confirm(&mut self) {
        let Some(pane) = &self.sftp_pane else {
            return;
        };
        let selected_count = match pane.side {
            PaneSide::Local => pane.local_selection.len(),
            PaneSide::Remote => pane.remote_selection.len(),
        };
        if selected_count == 0 {
            return;
        }
        self.sftp_delete_confirm = Some(SftpDeleteConfirmState {
            side: pane.side,
            selected_count,
        });
    }

    fn exit_sftp(&mut self) {
        self.sftp_delete_confirm = None;
        self.mode = AppMode::Main;
    }

    fn clear_transfer_queue(&mut self) {
        if let Some(transfer) = &self.active_transfer {
            transfer.cancel.cancel();
        }
        self.active_transfer = None;
        self.active_recursive_transfer = None;
        self.queued_transfers.clear();
        self.queue_completed_count = 0;
        self.queue_total_count = 0;
        self.recursive_conflict_prompt = None;
        self.recursive_failure_prompt = None;
        self.conflict_resolution = ConflictResolutionState::default();
    }

    fn clear_connection_state(&mut self) {
        self.clear_transfer_queue();
        self.sftp_client = None;
        self.sftp_pane = None;
        self.remote_edit_session = None;
        self.current_host_alias = None;
        self.sftp_delete_confirm = None;
    }

    fn toggle_pane(&mut self) {
        if let Some(pane) = &mut self.sftp_pane {
            pane.side = match pane.side {
                PaneSide::Local => PaneSide::Remote,
                PaneSide::Remote => PaneSide::Local,
            };
        }
    }

    fn pane_select_prev(&mut self) {
        if let Some(pane) = &mut self.sftp_pane {
            let i = pane.selected_index();
            pane.active_list_state_mut()
                .select(Some(i.saturating_sub(1)));
        }
    }

    fn pane_select_next(&mut self) {
        if let Some(pane) = &mut self.sftp_pane {
            let len = match pane.side {
                PaneSide::Local => pane.local_entries.len(),
                PaneSide::Remote => pane.remote_entries.len(),
            };
            if len == 0 {
                return;
            }
            let i = pane.selected_index();
            pane.active_list_state_mut()
                .select(Some((i + 1).min(len - 1)));
        }
    }

    fn apply_search(&mut self) {
        let scoped_indices = if self.show_folder_sidebar {
            self.folder_search_prefix()
                .map(|path| hosts_in_path(path, &self.hosts))
                .unwrap_or_default()
        } else {
            (0..self.hosts.len()).collect()
        };

        let scoped_hosts: Vec<Host> = scoped_indices
            .iter()
            .map(|&idx| self.hosts[idx].clone())
            .collect();
        let result = crate::utils::fuzzy::search(
            &self.search_query,
            &scoped_hosts,
            &self.connection_history,
        );
        self.filtered_indices = result.into_iter().map(|i| scoped_indices[i]).collect();
        let sel = if self.filtered_indices.is_empty() {
            None
        } else {
            Some(0)
        };
        self.list_state.select(sel);
        self.on_selection_changed();
    }

    fn toggle_folder_sidebar(&mut self) {
        if self.show_folder_sidebar {
            self.show_folder_sidebar = false;
            if self.main_focus == MainFocus::Directory {
                self.main_focus = MainFocus::HostList;
            }
            if self.main_focus_before_search == MainFocus::Directory {
                self.main_focus_before_search = MainFocus::HostList;
            }
        } else {
            if self.folder_view_state.is_none() {
                self.folder_view_state = Some(FolderViewState::new(&self.hosts));
            }
            if let Some(fv) = self.folder_view_state.as_mut() {
                fv.jump = None;
            }
            self.show_folder_sidebar = true;
            self.main_focus = MainFocus::Directory;
        }
        self.apply_search();
    }

    fn directory_select_previous(&mut self) {
        if let Some(fv) = self.folder_view_state.as_mut() {
            if fv.sel_a > 0 {
                fv.sel_a -= 1;
            }
            fv.update_col_b();
        }
        self.apply_search();
    }

    fn directory_select_next(&mut self) {
        if let Some(fv) = self.folder_view_state.as_mut() {
            if fv.sel_a + 1 < fv.col_a.len() {
                fv.sel_a += 1;
            }
            fv.update_col_b();
        }
        self.apply_search();
    }

    fn directory_enter(&mut self) {
        if let Some(fv) = self.folder_view_state.as_mut() {
            let selected = fv.selected_path().to_string();
            let children = fv
                .tree
                .get(&selected)
                .map(|node| node.children.clone())
                .unwrap_or_default();
            if !children.is_empty() {
                fv.col_a = children;
                fv.sel_a = 0;
                fv.depth += 1;
                fv.update_col_b();
            }
        }
        self.apply_search();
    }

    fn directory_back(&mut self) {
        if let Some(fv) = self.folder_view_state.as_mut() {
            let current = fv.selected_path().to_string();
            if current == "/" {
                return;
            }

            let parent = parent_path(&current);
            if parent == "/" {
                fv.col_a = level_1_paths(&fv.tree);
                fv.sel_a = fv.col_a.iter().position(|path| path == "/").unwrap_or(0);
                fv.depth = 0;
            } else {
                let grandparent = parent_path(&parent);
                fv.col_a = fv
                    .tree
                    .get(&grandparent)
                    .map(|node| node.children.clone())
                    .unwrap_or_else(|| level_1_paths(&fv.tree));
                fv.sel_a = fv
                    .col_a
                    .iter()
                    .position(|path| path == &parent)
                    .unwrap_or(0);
                fv.depth = fv.depth.saturating_sub(1);
            }
            fv.update_col_b();
        }
        self.apply_search();
    }

    fn set_status(&mut self, msg: String) {
        self.status_msg = Some((msg, Instant::now()));
    }

    pub fn main_status_message(&self) -> Option<&str> {
        self.pending_connection
            .as_ref()
            .map(|pending| pending.status.as_str())
            .or_else(|| self.status_msg.as_ref().map(|(msg, _)| msg.as_str()))
    }

    fn start_pending_connection(&mut self, kind: PendingConnectionKind, host: Host) -> Result<()> {
        let route = self.resolve_connection_route(&host)?;
        let connect_host = match &route {
            ConnectionRoute::Direct(target) => target.clone(),
            ConnectionRoute::ViaProxyJump { bastion, .. } => bastion.clone(),
        };
        let status = match (&kind, &route) {
            (PendingConnectionKind::Ssh, ConnectionRoute::Direct(target)) => {
                format!("Connecting to {}:{}...", target.hostname, target.port)
            }
            (PendingConnectionKind::Sftp, ConnectionRoute::Direct(target)) => {
                format!(
                    "Connecting to {}:{} for SFTP...",
                    target.hostname, target.port
                )
            }
            (PendingConnectionKind::Ssh, ConnectionRoute::ViaProxyJump { bastion, target }) => {
                format!(
                    "Connecting to {}:{} via {}...",
                    target.hostname, target.port, bastion.alias
                )
            }
            (PendingConnectionKind::Sftp, ConnectionRoute::ViaProxyJump { bastion, target }) => {
                format!(
                    "Connecting to {}:{} for SFTP via {}...",
                    target.hostname, target.port, bastion.alias
                )
            }
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = ActiveSession::connect(&connect_host.hostname, connect_host.port).await;
            let _ = tx.send(result);
        });
        self.pending_connection = Some(PendingConnection {
            kind,
            host,
            route,
            status,
            rx,
        });
        Ok(())
    }

    fn resolve_connection_route(&self, host: &Host) -> Result<ConnectionRoute> {
        match host.proxy_jump.as_deref() {
            Some(jump_alias) => {
                let bastion = self
                    .hosts
                    .iter()
                    .find(|candidate| candidate.alias == jump_alias)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("proxy jump host '{jump_alias}' not found"))?;
                Ok(ConnectionRoute::ViaProxyJump {
                    bastion: Box::new(bastion),
                    target: Box::new(host.clone()),
                })
            }
            None => Ok(ConnectionRoute::Direct(Box::new(host.clone()))),
        }
    }

    async fn poll_pending_connection(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
    ) -> Result<()> {
        let Some(mut pending) = self.pending_connection.take() else {
            return Ok(());
        };

        match pending.rx.try_recv() {
            Ok(connect_result) => {
                let host = pending.host;
                let finish_result = match connect_result {
                    Ok(session) => {
                        self.finish_pending_connection(
                            terminal,
                            bus,
                            pending.kind,
                            &host,
                            pending.route,
                            session,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };

                if let Err(error) = finish_result {
                    self.connection_history.record(&host.alias);
                    let prefix = match pending.kind {
                        PendingConnectionKind::Ssh => "Connection failed",
                        PendingConnectionKind::Sftp => "SFTP failed",
                    };
                    self.set_status(format!("{prefix}: {error}"));
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                self.pending_connection = Some(pending);
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.set_status("Connection failed: background task closed unexpectedly".into());
            }
        }

        Ok(())
    }

    async fn finish_pending_connection(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        kind: PendingConnectionKind,
        host: &Host,
        route: ConnectionRoute,
        session: ActiveSession,
    ) -> Result<()> {
        match route {
            ConnectionRoute::Direct(_) => match kind {
                PendingConnectionKind::Ssh => {
                    self.finish_ssh_connect(terminal, bus, host, session).await
                }
                PendingConnectionKind::Sftp => {
                    self.finish_sftp_connect(terminal, bus, host, session).await
                }
            },
            ConnectionRoute::ViaProxyJump { bastion, target } => {
                let mut bastion_session = self
                    .authenticate_connected_session(terminal, bus, &bastion, session)
                    .await?;
                let target_handle = proxy_jump::connect_via_authenticated_bastion(
                    &mut bastion_session.handle,
                    &target,
                    ClientHandler::default(),
                )
                .await?;
                let target_session = ActiveSession {
                    handle: target_handle,
                    channel: None,
                };
                match kind {
                    PendingConnectionKind::Ssh => {
                        self.finish_ssh_connect(terminal, bus, &target, target_session)
                            .await
                    }
                    PendingConnectionKind::Sftp => {
                        self.finish_sftp_connect(terminal, bus, &target, target_session)
                            .await
                    }
                }
            }
        }
    }

    fn normalize_queue_counters(&mut self) {
        if self.active_transfer.is_none()
            && self.active_recursive_transfer.is_none()
            && self.queued_transfers.is_empty()
        {
            self.queue_completed_count = 0;
            self.queue_total_count = 0;
        }
    }

    async fn enqueue_transfers(&mut self, transfers: Vec<QueuedTransfer>) -> Result<()> {
        if transfers.is_empty() {
            return Ok(());
        }

        let was_idle = self.active_transfer.is_none()
            && self.active_recursive_transfer.is_none()
            && self.queued_transfers.is_empty();
        if was_idle {
            self.queue_completed_count = 0;
            self.queue_total_count = transfers.len();
        } else {
            self.queue_total_count += transfers.len();
        }
        self.queued_transfers.extend(transfers);

        if was_idle {
            self.start_next_queued_transfer().await?;
        }

        Ok(())
    }

    async fn poll_transfer_queue(&mut self) -> Result<()> {
        let mut recursive_result = None;
        if let Some(tr) = &mut self.active_transfer {
            while let Ok(mut prog) = tr.rx.try_recv() {
                if let Some(recursive) = self.active_recursive_transfer.as_ref() {
                    recursive.sync_transfer_progress(&mut prog);
                }
                tr.progress = prog;
            }
            let done = matches!(
                tr.progress.state,
                TransferState::Completed | TransferState::Failed(_) | TransferState::Cancelled
            );
            if done && self.active_recursive_transfer.is_some() {
                recursive_result = Some(tr.progress.clone());
            } else if done {
                if tr.needs_refresh {
                    tr.needs_refresh = false;
                    match tr.dir {
                        TransferDir::Download => self.trigger_refresh_local = true,
                        TransferDir::Upload => self.trigger_refresh_remote = true,
                    }
                }
                if tr.done_at.is_none() {
                    tr.done_at = Some(Instant::now());
                }
                if tr
                    .done_at
                    .map(|t| t.elapsed().as_secs() >= 3)
                    .unwrap_or(false)
                {
                    self.queue_completed_count += 1;
                    self.active_transfer = None;
                }
            }
        }
        if let Some(progress) = recursive_result {
            let was_terminal = matches!(
                progress.state,
                TransferState::Completed | TransferState::Cancelled
            );
            self.active_transfer = None;
            self.handle_recursive_transfer_result(progress);
            if was_terminal && self.active_recursive_transfer.is_none() {
                self.queue_completed_count += 1;
            }
        }
        if self.active_transfer.is_none() {
            self.poll_recursive_transfer().await?;
            if self.active_transfer.is_none() && self.active_recursive_transfer.is_none() {
                self.start_next_queued_transfer().await?;
            }
        }
        Ok(())
    }

    async fn start_next_queued_transfer(&mut self) -> Result<()> {
        if self.active_transfer.is_some() || self.active_recursive_transfer.is_some() {
            return Ok(());
        }

        let Some(transfer) = self.queued_transfers.pop_front() else {
            self.normalize_queue_counters();
            return Ok(());
        };

        match transfer {
            QueuedTransfer::SingleFile {
                dir,
                display_name,
                local_path,
                remote_path,
                total_bytes,
            } => {
                let session = self.open_transfer_session().await?;
                let (verb, handle) = match dir {
                    TransferDir::Download => (
                        "Downloading",
                        download(session, remote_path, local_path, total_bytes),
                    ),
                    TransferDir::Upload => ("Uploading", upload(session, local_path, remote_path)?),
                };
                self.active_transfer = Some(ActiveTransfer {
                    verb,
                    dir,
                    progress: TransferProgress {
                        filename: display_name,
                        total_bytes,
                        transferred_bytes: 0,
                        state: TransferState::InProgress,
                        current_file_index: 1,
                        total_files: 1,
                    },
                    rx: handle.rx,
                    cancel: handle.cancel,
                    done_at: None,
                    needs_refresh: true,
                });
            }
            QueuedTransfer::Recursive { display_name, plan } => {
                self.conflict_resolution = ConflictResolutionState::default();
                self.recursive_conflict_prompt = None;
                self.recursive_failure_prompt = None;
                self.active_recursive_transfer = Some(ActiveRecursiveTransfer::new(plan));
                self.set_status(format!("Started background transfer for {display_name}"));
            }
        }

        Ok(())
    }

    async fn start_remote_edit(&mut self) -> Result<()> {
        let Some(pane) = &self.sftp_pane else {
            return Ok(());
        };
        if pane.side != PaneSide::Remote {
            return Ok(());
        }
        let idx = pane.selected_index();
        let Some(entry) = pane.remote_entries.get(idx).cloned() else {
            return Ok(());
        };
        if entry.is_dir {
            self.set_status("Recursive remote directory download is not wired yet".into());
            return Ok(());
        }
        let remote_path = format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name);
        let workspace = tempfile::tempdir()?;
        let local_path = workspace.path().join(&entry.name);

        let Some(client) = &self.sftp_client else {
            return Ok(());
        };
        client
            .download_file_to_path(&remote_path, &local_path)
            .await?;

        let initial_fingerprint = fingerprint_file(&local_path)?;
        let (watcher, watch_rx) = start_remote_edit_watcher(workspace.path())?;
        open::open_path(&local_path)?;

        let mut session = RemoteEditSession::with_runtime(
            remote_path,
            local_path,
            workspace,
            initial_fingerprint,
            Some(watcher),
            watch_rx,
        );
        session.mark_watching();
        let file_name = session.display_name();
        self.remote_edit_session = Some(session);
        self.set_status(format!("Opened {file_name} for local editing"));
        Ok(())
    }

    async fn poll_remote_edit(&mut self) -> Result<()> {
        let Some(mut session) = self.remote_edit_session.take() else {
            return Ok(());
        };

        while session.watch_rx.try_recv().is_ok() {}

        let fingerprint = match fingerprint_file(&session.local_path) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                session.mark_upload_failed(error.to_string());
                self.set_status(format!("Remote edit file is unavailable: {error}"));
                self.remote_edit_session = Some(session);
                return Ok(());
            }
        };

        if session.should_upload(&fingerprint) {
            session.mark_uploading();
            let file_name = session.display_name();
            let upload_result = if let Some(client) = &self.sftp_client {
                client
                    .upload_file_from_path(&session.local_path, &session.remote_path)
                    .await
            } else {
                bail!("SFTP session is not available")
            };

            match upload_result {
                Ok(()) => {
                    session.mark_uploaded(fingerprint);
                    self.trigger_refresh_remote = true;
                    self.set_status(format!("Uploaded {file_name}"));
                }
                Err(error) => {
                    session.mark_upload_failed(error.to_string());
                    self.set_status(format!("Auto upload failed: {error}"));
                }
            }
        }

        self.remote_edit_session = Some(session);
        Ok(())
    }

    fn remote_edit_status(&self) -> Option<&str> {
        self.status_msg.as_ref().map(|(msg, _)| msg.as_str())
    }

    fn reset_probe(&mut self) {
        self.hover_host_id = None;
        self.hover_since = None;
        self.probe_rx = None;
        self.probe_result = None;
    }

    fn on_selection_changed(&mut self) {
        let new_id = self
            .filtered_indices
            .get(self.list_state.selected().unwrap_or(0))
            .map(|&i| self.hosts[i].id.clone());

        if self.hover_host_id != new_id {
            self.reset_probe();
            self.hover_host_id = new_id;
            self.hover_since = Some(Instant::now());
        }
    }

    fn open_import_view(&mut self) {
        let ssh_hosts = crate::config::ssh_config::import_ssh_config()
            .map(|(h, _)| h)
            .unwrap_or_default();
        if ssh_hosts.is_empty() {
            return;
        }
        self.import_state = Some(ImportViewState::new(ssh_hosts, &self.hosts));
        self.mode = AppMode::ImportSshConfig;
    }

    fn rebuild_forwarding_state(&mut self) {
        let mut next = ForwardingViewState::new(&self.hosts);
        if let Some(previous) = &self.forwarding_state {
            let selected_host_id = previous
                .selected_host_idx()
                .and_then(|host_idx| self.hosts.get(host_idx))
                .map(|host| host.id.clone());
            let selected_rule_id = previous
                .selected_host_idx()
                .and_then(|host_idx| {
                    previous
                        .rule_list_state
                        .selected()
                        .map(|rule_idx| (host_idx, rule_idx))
                })
                .and_then(|(host_idx, rule_idx)| self.hosts.get(host_idx)?.forwards.get(rule_idx))
                .map(|rule| rule.id.clone());

            next.focus = previous.focus;
            next.statuses = previous.statuses.clone();

            if let Some(host_id) = selected_host_id
                && let Some(host_pos) = next
                    .host_indices
                    .iter()
                    .position(|&host_idx| self.hosts[host_idx].id == host_id)
            {
                next.host_list_state.select(Some(host_pos));
            }

            if let Some(rule_id) = selected_rule_id
                && let Some(host_idx) = next.selected_host_idx()
                && let Some(rule_pos) = self.hosts[host_idx]
                    .forwards
                    .iter()
                    .position(|rule| rule.id == rule_id)
            {
                next.rule_list_state.select(Some(rule_pos));
            }
        }
        self.forwarding_state = Some(next);
    }

    fn handle_forwarding_key(&mut self, k: KeyEvent) {
        use crate::tui::views::forward_edit::{EditField as ForwardEditField, ForwardEditState};

        if self.forward_edit.is_some() {
            self.handle_forward_edit_key(k);
            return;
        }

        let Some(state) = &mut self.forwarding_state else {
            return;
        };

        match (k.code, k.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) | (KeyCode::Esc, _) => {
                self.mode = AppMode::Main;
                self.forwarding_state = None;
                self.forward_edit = None;
            }
            (KeyCode::Tab, _) => {
                state.focus = match state.focus {
                    ForwardingFocus::HostList => {
                        state.rule_list_state = ListState::default();
                        if let Some(host_i) = state.selected_host_idx() {
                            let count = self.hosts[host_i].forwards.len();
                            if count > 0 {
                                state.rule_list_state.select(Some(0));
                            }
                        }
                        ForwardingFocus::RuleList
                    }
                    ForwardingFocus::RuleList => ForwardingFocus::HostList,
                };
            }
            (KeyCode::Up, _) => match state.focus {
                ForwardingFocus::HostList => {
                    let count = state.host_indices.len();
                    if count > 0 {
                        let current = state.host_list_state.selected().unwrap_or(0);
                        state
                            .host_list_state
                            .select(Some(current.saturating_sub(1)));
                        state.rule_list_state = ListState::default();
                    }
                }
                ForwardingFocus::RuleList => {
                    let current = state.rule_list_state.selected().unwrap_or(0);
                    state
                        .rule_list_state
                        .select(Some(current.saturating_sub(1)));
                }
            },
            (KeyCode::Down, _) => match state.focus {
                ForwardingFocus::HostList => {
                    let count = state.host_indices.len();
                    if count > 0 {
                        let current = state.host_list_state.selected().unwrap_or(0);
                        state
                            .host_list_state
                            .select(Some((current + 1).min(count - 1)));
                        state.rule_list_state = ListState::default();
                    }
                }
                ForwardingFocus::RuleList => {
                    let count = state
                        .selected_host_idx()
                        .map(|idx| self.hosts[idx].forwards.len())
                        .unwrap_or(0);
                    if count > 0 {
                        let current = state.rule_list_state.selected().unwrap_or(0);
                        state
                            .rule_list_state
                            .select(Some((current + 1).min(count - 1)));
                    }
                }
            },
            (KeyCode::Enter, _) if state.focus == ForwardingFocus::RuleList => {
                if let Some(host_idx) = state.selected_host_idx()
                    && let Some(rule_idx) = state.rule_list_state.selected()
                    && let Some(rule) = self.hosts[host_idx].forwards.get(rule_idx)
                {
                    let rule_id = rule.id.clone();
                    let is_active = state
                        .statuses
                        .iter()
                        .find(|status| status.id == rule_id)
                        .map(|status| status.state.is_active())
                        .unwrap_or(false);
                    tokio::spawn(async move {
                        if is_active {
                            let _ = crate::tunnel::client::daemon_stop(&rule_id).await;
                        } else {
                            let _ = crate::tunnel::client::daemon_start(&rule_id).await;
                        }
                    });
                }
            }
            (KeyCode::Char('n'), KeyModifiers::NONE) => {
                if let Some(host_idx) = state.selected_host_idx() {
                    let host = &self.hosts[host_idx];
                    self.forward_edit =
                        Some(ForwardEditState::new(host.id.clone(), host.alias.clone()));
                }
            }
            (KeyCode::Char('e'), KeyModifiers::NONE)
                if state.focus == ForwardingFocus::RuleList =>
            {
                if let Some(host_idx) = state.selected_host_idx()
                    && let Some(rule_idx) = state.rule_list_state.selected()
                    && let Some(rule) = self.hosts[host_idx].forwards.get(rule_idx)
                {
                    let host = &self.hosts[host_idx];
                    self.forward_edit = Some(ForwardEditState::from_rule(
                        host.id.clone(),
                        host.alias.clone(),
                        rule,
                    ));
                }
            }
            (KeyCode::Char('d'), KeyModifiers::NONE)
                if state.focus == ForwardingFocus::RuleList =>
            {
                if let Some(host_idx) = state.selected_host_idx()
                    && let Some(rule_idx) = state.rule_list_state.selected()
                    && rule_idx < self.hosts[host_idx].forwards.len()
                {
                    self.hosts[host_idx].forwards.remove(rule_idx);
                    self.save_hosts_to_disk();
                    self.rebuild_forwarding_state();
                }
            }
            (KeyCode::Char(' '), KeyModifiers::NONE) => {
                if let Some(edit) = &mut self.forward_edit
                    && edit.focused == ForwardEditField::AutoStart
                {
                    edit.auto_start = !edit.auto_start;
                }
            }
            _ => {}
        }
    }

    fn handle_forward_edit_key(&mut self, k: KeyEvent) {
        use crate::tui::views::forward_edit::EditField;

        let Some(edit) = &mut self.forward_edit else {
            return;
        };

        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                self.forward_edit = None;
            }
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => match edit.validate() {
                Ok(rule) => {
                    let host_id = edit.host_id.clone();
                    let is_new = edit.forward_id.is_none();
                    if let Some(host) = self.hosts.iter_mut().find(|host| host.id == host_id) {
                        if is_new {
                            host.forwards.push(rule);
                        } else if let Some(existing) = host
                            .forwards
                            .iter_mut()
                            .find(|existing| existing.id == rule.id)
                        {
                            *existing = rule;
                        }
                    }
                    self.save_hosts_to_disk();
                    self.rebuild_forwarding_state();
                    self.forward_edit = None;
                }
                Err(message) => {
                    edit.error = Some(message);
                }
            },
            (KeyCode::Tab, _) | (KeyCode::Down, _) => {
                edit.focused = edit.focused.next(edit.kind_idx);
                edit.error = None;
            }
            (KeyCode::BackTab, _) | (KeyCode::Up, _) => {
                edit.focused = edit.focused.prev(edit.kind_idx);
                edit.error = None;
            }
            (KeyCode::Left, _) if edit.focused == EditField::Kind => {
                edit.kind_idx = edit.kind_idx.saturating_sub(1);
                if edit.kind_idx == 2 && edit.focused == EditField::RemotePort {
                    edit.focused = EditField::AutoStart;
                }
            }
            (KeyCode::Right, _) if edit.focused == EditField::Kind => {
                edit.kind_idx = (edit.kind_idx + 1).min(2);
            }
            (KeyCode::Char(' '), KeyModifiers::NONE) if edit.focused == EditField::Kind => {
                edit.kind_idx = (edit.kind_idx + 1) % 3;
            }
            (KeyCode::Char(' '), KeyModifiers::NONE) if edit.focused == EditField::AutoStart => {
                edit.auto_start = !edit.auto_start;
            }
            (KeyCode::Char(c), KeyModifiers::NONE) => {
                let target = match edit.focused {
                    EditField::Name => Some(&mut edit.name),
                    EditField::LocalPort => Some(&mut edit.local_port),
                    EditField::RemoteHost => Some(&mut edit.remote_host),
                    EditField::RemotePort => Some(&mut edit.remote_port),
                    _ => None,
                };
                if let Some(target) = target {
                    target.push(c);
                    edit.error = None;
                }
            }
            (KeyCode::Backspace, _) => {
                let target = match edit.focused {
                    EditField::Name => Some(&mut edit.name),
                    EditField::LocalPort => Some(&mut edit.local_port),
                    EditField::RemoteHost => Some(&mut edit.remote_host),
                    EditField::RemotePort => Some(&mut edit.remote_port),
                    _ => None,
                };
                if let Some(target) = target {
                    target.pop();
                    edit.error = None;
                }
            }
            _ => {}
        }
    }

    fn save_hosts_to_disk(&self) {
        let mut metadata = self.metadata.clone();
        metadata.import_prompted = self.import_prompted;
        #[cfg(test)]
        let config_path = self
            .test_config_dir
            .as_ref()
            .map(|dir| dir.path().join("hosts.toml"))
            .unwrap_or_else(store::config_path);
        #[cfg(not(test))]
        let config_path = store::config_path();
        let _ = store::save_store(
            &config_path,
            &store::HostStore {
                metadata,
                hosts: self.hosts.clone(),
            },
        );
    }

    fn password_prompt_title(&mut self, user: &str, hostname: &str, account: &str) -> String {
        let title = format!("Password for {}@{}: ", user, hostname);
        let Some(failure) = self
            .metadata
            .secret_save_failures
            .iter()
            .find(|failure| failure.account == account)
        else {
            return title;
        };

        format!(
            "Password was not saved last time: {}\n{title}",
            failure.reason
        )
    }

    fn record_secret_save_failure(&mut self, account: String, error: &SecretError) {
        let mut reason = error.user_message().to_string();
        if cfg!(target_os = "linux") && matches!(error, SecretError::Unavailable(_)) {
            reason.push_str(
                ". Install and unlock a Secret Service provider such as gnome-keyring or kwallet.",
            );
        }

        self.metadata.upsert_secret_failure(account, reason);
        self.save_hosts_to_disk();
    }

    fn clear_secret_save_failure(&mut self, account: &str) {
        if self.metadata.take_secret_failure(account).is_some() {
            self.save_hosts_to_disk();
        }
    }

    fn key_passphrase_prompt_title(&mut self, path: &str, account: &str) -> String {
        let title = format!("Key passphrase ({}): ", path);
        let Some(failure) = self
            .metadata
            .secret_save_failures
            .iter()
            .find(|failure| failure.account == account)
        else {
            return title;
        };

        format!(
            "Password was not saved last time: {}\n{title}",
            failure.reason
        )
    }

    fn handle_edit_input(&mut self, k: KeyEvent) {
        let Some(focused_field) = self.edit_draft.as_ref().map(|draft| draft.focused_field) else {
            return;
        };
        let proxy_jump_candidates = if focused_field == EditField::ProxyJump {
            let proxy_jump_aliases: Vec<String> =
                self.hosts.iter().map(|host| host.alias.clone()).collect();
            self.edit_draft
                .as_ref()
                .map(|draft| edit_view::proxy_jump_candidates(draft, &proxy_jump_aliases))
                .unwrap_or_default()
        } else {
            vec![]
        };

        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                if focused_field == EditField::ProxyJump {
                    let draft = self.edit_draft.as_mut().unwrap();
                    if draft.proxy_jump_candidates_open && !proxy_jump_candidates.is_empty() {
                        draft.proxy_jump_candidates_open = false;
                        draft.proxy_jump_candidate_sel = 0;
                        return;
                    }
                }
                self.edit_draft = None;
                self.mode = AppMode::Main;
                self.on_selection_changed();
                return;
            }
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                self.try_save_edit();
                return;
            }
            (KeyCode::Tab, _) | (KeyCode::Down, _) => {
                let draft = self.edit_draft.as_mut().unwrap();
                if draft.focused_field == EditField::Tags {
                    if !draft.tags.candidates.is_empty() {
                        draft.tags.handle_down();
                        return;
                    }
                    draft.tags.commit_pending();
                }
                draft.proxy_jump_candidates_open = false;
                draft.focused_field = draft.focused_field.next();
                return;
            }
            (KeyCode::BackTab, _) | (KeyCode::Up, _) => {
                let draft = self.edit_draft.as_mut().unwrap();
                if draft.focused_field == EditField::Tags {
                    if !draft.tags.candidates.is_empty() {
                        draft.tags.handle_up();
                        return;
                    }
                    draft.tags.commit_pending();
                }
                draft.proxy_jump_candidates_open = false;
                draft.focused_field = draft.focused_field.prev();
                return;
            }
            _ => {}
        }

        if focused_field == EditField::ProxyJump {
            let draft = self.edit_draft.as_mut().unwrap();

            match (k.code, k.modifiers) {
                (KeyCode::Down, _) if !proxy_jump_candidates.is_empty() => {
                    draft.proxy_jump_candidates_open = true;
                    draft.proxy_jump_candidate_sel =
                        (draft.proxy_jump_candidate_sel + 1).min(proxy_jump_candidates.len() - 1);
                    return;
                }
                (KeyCode::Up, _) if !proxy_jump_candidates.is_empty() => {
                    draft.proxy_jump_candidates_open = true;
                    draft.proxy_jump_candidate_sel =
                        draft.proxy_jump_candidate_sel.saturating_sub(1);
                    return;
                }
                (KeyCode::Enter, _) if !proxy_jump_candidates.is_empty() => {
                    if let Some(candidate) =
                        proxy_jump_candidates.get(draft.proxy_jump_candidate_sel)
                    {
                        draft.proxy_jump = candidate.clone();
                        draft.proxy_jump_candidate_sel = 0;
                        draft.proxy_jump_candidates_open = false;
                    }
                    return;
                }
                (KeyCode::Backspace, _) => {
                    draft.proxy_jump.pop();
                    draft.proxy_jump_candidate_sel = 0;
                    draft.proxy_jump_candidates_open = !draft.proxy_jump.is_empty();
                    return;
                }
                (KeyCode::Char(c), KeyModifiers::NONE)
                | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                    draft.proxy_jump.push(c);
                    draft.proxy_jump_candidate_sel = 0;
                    draft.proxy_jump_candidates_open = true;
                    return;
                }
                _ => {}
            }
        }

        let draft = self.edit_draft.as_mut().unwrap();

        if draft.focused_field == EditField::Tags {
            let all_tags: Vec<String> = {
                let mut t: Vec<String> = self
                    .hosts
                    .iter()
                    .flat_map(|h| h.tags.iter().cloned())
                    .collect();
                t.sort();
                t.dedup();
                t
            };
            let draft = self.edit_draft.as_mut().unwrap();
            match (k.code, k.modifiers) {
                (KeyCode::Left, _) => draft.tags.handle_left(),
                (KeyCode::Right, _) => draft.tags.handle_right(),
                (KeyCode::Backspace, _) => draft.tags.handle_backspace(&all_tags),
                (KeyCode::Enter, _) | (KeyCode::Tab, _) => draft.tags.confirm_input(),
                (KeyCode::Esc, _) => draft.tags.cancel_input(),
                (KeyCode::Char(c), KeyModifiers::NONE)
                | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                    draft.tags.handle_char(c, &all_tags);
                }
                _ => {}
            }
            return;
        }

        let draft = self.edit_draft.as_mut().unwrap();
        // Text field handling
        match k.code {
            KeyCode::Backspace => {
                if let Some(v) = draft.active_text_mut() {
                    v.pop();
                }
            }
            KeyCode::Char(c)
                if k.modifiers == KeyModifiers::NONE || k.modifiers == KeyModifiers::SHIFT =>
            {
                if draft.focused_field == EditField::Port && !c.is_ascii_digit() {
                    return;
                }
                if let Some(v) = draft.active_text_mut() {
                    v.push(c);
                }
            }
            _ => {}
        }
    }

    fn try_save_edit(&mut self) {
        let Some(draft) = self.edit_draft.as_mut() else {
            return;
        };
        draft.error = None;

        if draft.is_new {
            let alias = draft.alias.trim().to_string();
            if self.hosts.iter().any(|h| h.alias == alias) {
                draft.error = Some("Alias already exists".into());
                return;
            }
        }

        match edit_view::validate(draft) {
            Err(e) => {
                draft.error = Some(e);
            }
            Ok(()) => {
                let proxy_jump = draft.proxy_jump.trim();
                if !proxy_jump.is_empty() {
                    if proxy_jump == draft.alias.trim() {
                        draft.error = Some("Proxy Jump cannot reference the current host".into());
                        return;
                    }
                    if !self.hosts.iter().any(|host| host.alias == proxy_jump) {
                        draft.error = Some("Proxy Jump must match an existing host alias".into());
                        return;
                    }
                }
                let host = edit_view::build_host(draft);
                if draft.is_new {
                    self.hosts.push(host);
                } else {
                    let id = draft.original_id.clone().unwrap_or_default();
                    if let Some(h) = self.hosts.iter_mut().find(|h| h.id == id) {
                        *h = host;
                    }
                }
                self.apply_search();
                self.save_hosts_to_disk();
                self.edit_draft = None;
                self.mode = AppMode::Main;
            }
        }
    }

    fn handle_confirm_delete(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm_delete = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let id = self.hosts[idx].id.clone();
                    self.hosts.retain(|h| h.id != id);
                    self.apply_search();
                    self.save_hosts_to_disk();
                }
            }
            _ => {
                self.confirm_delete = false;
            }
        }
    }

    fn handle_sftp_delete_confirm(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.sftp_delete_confirm = None;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.sftp_delete_confirm = None;
                self.trigger_sftp_delete = true;
            }
            _ => {}
        }
    }

    fn handle_import_prompt_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.show_import_prompt = false;
                self.open_import_view();
            }
            _ => {
                self.show_import_prompt = false;
                self.import_prompted = true;
                self.save_hosts_to_disk();
            }
        }
    }

    fn handle_recursive_conflict_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if let Some(prompt) = self.recursive_conflict_prompt.as_mut() {
                    prompt.toggle_apply_to_remaining();
                }
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                self.apply_recursive_conflict_choice(FileConflictChoice::Overwrite);
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.apply_recursive_conflict_choice(FileConflictChoice::Skip);
            }
            KeyCode::Esc => {
                self.recursive_conflict_prompt = None;
                self.active_recursive_transfer = None;
                self.set_status("Recursive transfer cancelled".into());
            }
            _ => {}
        }
    }

    fn apply_recursive_conflict_choice(&mut self, choice: FileConflictChoice) {
        let Some(prompt) = self.recursive_conflict_prompt.take() else {
            return;
        };
        self.conflict_resolution
            .apply_choice(choice, prompt.apply_to_remaining);
        if matches!(choice, FileConflictChoice::Skip)
            && let Some(transfer) = self.active_recursive_transfer.as_mut()
        {
            transfer.skip_pending_file();
        }
    }

    fn handle_recursive_failure_key(&mut self, k: KeyEvent) {
        let choice = match k.code {
            KeyCode::Char('r') | KeyCode::Char('R') => Some(FailureChoice::Retry),
            KeyCode::Char('s') | KeyCode::Char('S') => Some(FailureChoice::Skip),
            KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Esc => Some(FailureChoice::Cancel),
            _ => None,
        };
        let Some(choice) = choice else {
            return;
        };
        let Some(mut state) = self.recursive_failure_prompt.take() else {
            return;
        };
        state.apply(choice);
        if state.should_skip_current_file
            && let Some(transfer) = self.active_recursive_transfer.as_mut()
        {
            transfer.skip_pending_file();
        }
        if state.should_cancel_task {
            self.active_recursive_transfer = None;
            self.set_status("Recursive transfer cancelled".into());
        }
    }

    fn handle_folder_input(&mut self, k: KeyEvent) {
        let Some(fv) = self.folder_view_state.as_mut() else {
            return;
        };

        // Jump overlay takes priority
        if fv.jump.is_some() {
            self.handle_folder_jump_key(k);
            return;
        }

        // Search bar takes priority
        if fv.search.is_some() {
            self.handle_folder_search_key(k);
            return;
        }

        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                self.folder_view_state = None;
                self.mode = AppMode::Main;
            }

            (KeyCode::Char('j'), KeyModifiers::NONE) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                let all_cands = jump_candidates("", &fv.tree);
                fv.jump = Some(JumpState {
                    input: String::new(),
                    candidates: all_cands,
                    sel: 0,
                });
            }

            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                let scope = fv.focused_path().to_string();
                fv.search = Some(SearchState {
                    scope_path: scope,
                    query: String::new(),
                });
            }

            (KeyCode::Up, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                match fv.focus {
                    FolderFocus::DirA => {
                        if fv.sel_a > 0 {
                            fv.sel_a -= 1;
                        }
                        fv.update_col_b();
                    }
                    FolderFocus::DirB => {
                        if fv.sel_b > 0 {
                            fv.sel_b -= 1;
                        }
                    }
                    FolderFocus::Hosts => {
                        if fv.host_sel > 0 {
                            fv.host_sel -= 1;
                        }
                    }
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Down, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                match fv.focus {
                    FolderFocus::DirA => {
                        if fv.sel_a + 1 < fv.col_a.len() {
                            fv.sel_a += 1;
                        }
                        fv.update_col_b();
                    }
                    FolderFocus::DirB => {
                        if fv.sel_b + 1 < fv.col_b.len() {
                            fv.sel_b += 1;
                        }
                    }
                    FolderFocus::Hosts => {
                        let max = self.folder_host_indices.len().saturating_sub(1);
                        let fv = self.folder_view_state.as_mut().unwrap();
                        if fv.host_sel < max {
                            fv.host_sel += 1;
                        }
                    }
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Enter, _) | (KeyCode::Right, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                match fv.focus {
                    FolderFocus::DirA => {
                        if !fv.col_b.is_empty() {
                            let new_col_a = fv.col_b.clone();
                            fv.col_a = new_col_a;
                            fv.sel_a = 0;
                            fv.depth += 1;
                            fv.update_col_b();
                        } else {
                            fv.focus = FolderFocus::Hosts;
                        }
                    }
                    FolderFocus::DirB => {
                        let children = fv
                            .tree
                            .get(fv.col_b.get(fv.sel_b).map(|s| s.as_str()).unwrap_or("/"))
                            .map(|n| n.children.clone())
                            .unwrap_or_default();
                        if !children.is_empty() {
                            fv.col_a = fv.col_b.clone();
                            fv.sel_a = fv.sel_b;
                            fv.depth += 1;
                            fv.update_col_b();
                            fv.focus = FolderFocus::DirA;
                        } else {
                            fv.focus = FolderFocus::Hosts;
                        }
                    }
                    FolderFocus::Hosts => {
                        let host_sel = fv.host_sel;
                        if let Some(&idx) = self.folder_host_indices.get(host_sel) {
                            self.folder_view_state = None;
                            self.mode = AppMode::Main;
                            self.trigger_connect = true;
                            if let Some(pos) = self.filtered_indices.iter().position(|&i| i == idx)
                            {
                                self.list_state.select(Some(pos));
                            }
                        }
                    }
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Backspace, _) | (KeyCode::Left, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                match fv.focus {
                    FolderFocus::DirA if fv.depth > 0 => {
                        let current_first = fv.col_a.first().cloned().unwrap_or_else(|| "/".into());
                        let parent = parent_path(&current_first);
                        let grandparent = parent_path(&parent);
                        fv.col_b = fv.col_a.clone();
                        fv.sel_b = fv.sel_a;
                        fv.col_a = fv
                            .tree
                            .get(&grandparent)
                            .map(|n| n.children.clone())
                            .unwrap_or_else(|| level_1_paths(&fv.tree));
                        fv.sel_a = fv.col_a.iter().position(|p| p == &parent).unwrap_or(0);
                        fv.depth -= 1;
                        fv.focus = FolderFocus::DirA;
                    }
                    FolderFocus::DirB | FolderFocus::Hosts => {
                        fv.focus = FolderFocus::DirA;
                    }
                    _ => {}
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Tab, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                fv.focus = match fv.focus {
                    FolderFocus::DirA => {
                        if !fv.col_b.is_empty() {
                            FolderFocus::DirB
                        } else {
                            FolderFocus::Hosts
                        }
                    }
                    FolderFocus::DirB => FolderFocus::Hosts,
                    FolderFocus::Hosts => FolderFocus::DirA,
                };
            }

            (KeyCode::BackTab, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                fv.focus = match fv.focus {
                    FolderFocus::DirA => FolderFocus::Hosts,
                    FolderFocus::DirB => FolderFocus::DirA,
                    FolderFocus::Hosts => {
                        if !fv.col_b.is_empty() {
                            FolderFocus::DirB
                        } else {
                            FolderFocus::DirA
                        }
                    }
                };
            }

            (KeyCode::Char('s'), KeyModifiers::NONE)
                if matches!(
                    self.folder_view_state.as_ref().map(|f| f.focus),
                    Some(FolderFocus::Hosts)
                ) =>
            {
                let host_sel = self.folder_view_state.as_ref().unwrap().host_sel;
                if let Some(&idx) = self.folder_host_indices.get(host_sel) {
                    self.folder_view_state = None;
                    self.mode = AppMode::Main;
                    self.trigger_sftp = true;
                    if let Some(pos) = self.filtered_indices.iter().position(|&i| i == idx) {
                        self.list_state.select(Some(pos));
                    }
                }
            }

            _ => {}
        }
    }

    fn refresh_folder_hosts(&mut self) {
        if let Some(fv) = &self.folder_view_state {
            self.folder_host_indices = hosts_in_path(fv.focused_path(), &self.hosts);
        }
    }

    fn handle_folder_jump_key(&mut self, k: KeyEvent) {
        let Some(fv) = self.folder_view_state.as_mut() else {
            return;
        };

        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                fv.jump = None;
            }

            (KeyCode::Backspace, _) => {
                if let Some(jump) = fv.jump.as_mut() {
                    jump.input.pop();
                    let q = jump.input.clone();
                    jump.candidates = jump_candidates(&q, &fv.tree);
                    jump.sel = 0;
                }
            }

            (KeyCode::Up, _) => {
                if let Some(jump) = fv.jump.as_mut()
                    && jump.sel > 0
                {
                    jump.sel -= 1;
                }
            }

            (KeyCode::Down, _) => {
                if let Some(jump) = fv.jump.as_mut()
                    && !jump.candidates.is_empty()
                {
                    jump.sel = (jump.sel + 1).min(jump.candidates.len() - 1);
                }
            }

            (KeyCode::Tab, _) => {
                // Path completion: fill input with highlighted candidate
                let candidate = fv
                    .jump
                    .as_ref()
                    .and_then(|j| j.candidates.get(j.sel))
                    .cloned();
                if let Some(candidate) = candidate
                    && let Some(jump) = fv.jump.as_mut()
                {
                    jump.input = candidate.clone();
                    jump.candidates = jump_candidates(&candidate, &fv.tree);
                    jump.sel = 0;
                }
            }

            (KeyCode::Enter, _) => {
                // Clone target before re-borrowing fv to avoid borrow conflict
                let target = fv
                    .jump
                    .as_ref()
                    .and_then(|j| j.candidates.get(j.sel))
                    .cloned();
                if let Some(target) = target {
                    fv.jump_to(&target);
                    if self.mode == AppMode::Main {
                        self.apply_search();
                    } else {
                        self.refresh_folder_hosts();
                    }
                } else {
                    if let Some(fv) = self.folder_view_state.as_mut() {
                        fv.jump = None;
                    }
                }
            }

            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                if let Some(jump) = fv.jump.as_mut() {
                    jump.input.push(c);
                    let q = jump.input.clone();
                    jump.candidates = jump_candidates(&q, &fv.tree);
                    jump.sel = 0;
                }
            }

            _ => {}
        }
    }

    fn handle_folder_search_key(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                if let Some(fv) = self.folder_view_state.as_mut() {
                    fv.search = None;
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Enter, _) => {
                if let Some(fv) = self.folder_view_state.as_mut() {
                    fv.search = None;
                }
                // Keep current filtered results
            }

            (KeyCode::Backspace, _) => {
                if let Some(fv) = self.folder_view_state.as_mut()
                    && let Some(search) = fv.search.as_mut()
                {
                    search.query.pop();
                }
                self.apply_scoped_search();
            }

            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                if let Some(fv) = self.folder_view_state.as_mut()
                    && let Some(search) = fv.search.as_mut()
                {
                    search.query.push(c);
                }
                self.apply_scoped_search();
            }

            _ => {}
        }
    }

    fn apply_scoped_search(&mut self) {
        let Some(fv) = &self.folder_view_state else {
            return;
        };
        let Some(search) = &fv.search else {
            self.refresh_folder_hosts();
            return;
        };

        let scope = search.scope_path.clone();
        let query = search.query.clone();

        let path_indices = hosts_in_path(&scope, &self.hosts);

        if query.is_empty() {
            self.folder_host_indices = path_indices;
            return;
        }

        // Build a scoped sub-list, fuzzy-search within it, map results back to original indices
        let scoped: Vec<(usize, crate::config::host::Host)> = path_indices
            .iter()
            .map(|&i| (i, self.hosts[i].clone()))
            .collect();
        let sub_hosts: Vec<crate::config::host::Host> =
            scoped.iter().map(|(_, h)| h.clone()).collect();
        let fuzzy_results =
            crate::utils::fuzzy::search(&query, &sub_hosts, &self.connection_history);
        self.folder_host_indices = fuzzy_results.into_iter().map(|fi| scoped[fi].0).collect();
    }

    fn handle_import_input(&mut self, k: KeyEvent) {
        let Some(state) = self.import_state.as_mut() else {
            return;
        };
        match k.code {
            KeyCode::Esc => {
                self.import_state = None;
                self.import_prompted = true;
                self.mode = AppMode::Main;
                self.save_hosts_to_disk();
                self.on_selection_changed();
            }
            KeyCode::Enter => {
                let selected = state.selected_hosts();
                for h in selected {
                    if !self.hosts.iter().any(|e| e.id == h.id) {
                        self.hosts.push(h);
                    }
                }
                self.apply_search();
                self.import_prompted = true;
                self.save_hosts_to_disk();
                self.import_state = None;
                self.mode = AppMode::Main;
            }
            KeyCode::Char(' ') => state.toggle_selected(),
            KeyCode::Char('a') => state.toggle_all(),
            KeyCode::Up => state.move_up(),
            KeyCode::Down => state.move_down(),
            _ => {}
        }
    }

    fn select_next(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let next = (i + 1).min(self.filtered_indices.len() - 1);
        self.list_state.select(Some(next));
        self.on_selection_changed();
    }

    fn select_previous(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        let prev = i.saturating_sub(1);
        self.list_state.select(Some(prev));
        self.on_selection_changed();
    }

    async fn finish_sftp_connect(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        host: &Host,
        session: ActiveSession,
    ) -> Result<()> {
        let session = self
            .authenticate_connected_session(terminal, bus, host, session)
            .await?;
        let client = SftpClient::open(&session).await?;
        let home = client.home_dir().await;
        let remote_entries = client.list_dir(&home).await.unwrap_or_default();
        let local_path = std::env::current_dir().unwrap_or_default();
        let local_entries = list_local(&local_path).unwrap_or_default();

        let mut pane = SftpPaneState::new(home);
        pane.remote_entries = remote_entries;
        pane.local_entries = local_entries;

        self.sftp_client = Some(client);
        self.sftp_pane = Some(pane);
        self.current_host_alias = Some(host.alias.clone());
        self.active_session = Some(session);
        self.mode = AppMode::Sftp;
        self.connection_history.record(&host.alias);
        Ok(())
    }

    async fn pane_enter(&mut self) -> Result<()> {
        let Some(pane) = &mut self.sftp_pane else {
            return Ok(());
        };
        let idx = pane.selected_index();

        match pane.side {
            PaneSide::Remote => {
                let entry = pane.remote_entries.get(idx).cloned();
                if let Some(e) = entry {
                    if !e.is_dir {
                        return Ok(());
                    }
                    let new_path = if e.name == ".." {
                        let p = std::path::Path::new(&pane.remote_path);
                        p.parent()
                            .map(|pp| pp.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "/".into())
                    } else if e.name == "." {
                        pane.remote_path.clone()
                    } else {
                        format!("{}/{}", pane.remote_path.trim_end_matches('/'), e.name)
                    };
                    if let Some(client) = &self.sftp_client {
                        match client.list_dir(&new_path).await {
                            Ok(entries) => {
                                let pane = self.sftp_pane.as_mut().unwrap();
                                pane.remote_path = new_path;
                                pane.remote_entries = entries;
                                pane.remote_list_state.select(Some(0));
                            }
                            Err(e) => eprintln!("[List directory failed: {e}]"),
                        }
                    }
                }
            }
            PaneSide::Local => {
                let entry = pane.local_entries.get(idx).cloned();
                if let Some(e) = entry {
                    if !e.is_dir {
                        return Ok(());
                    }
                    let new_path = if e.name == ".." {
                        pane.local_path
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| pane.local_path.clone())
                    } else {
                        pane.local_path.join(&e.name)
                    };
                    match list_local(&new_path) {
                        Ok(entries) => {
                            let pane = self.sftp_pane.as_mut().unwrap();
                            pane.local_path = new_path;
                            pane.local_entries = entries;
                            pane.local_list_state.select(Some(0));
                        }
                        Err(e) => eprintln!("[List local directory failed: {e}]"),
                    }
                }
            }
        }
        Ok(())
    }

    async fn poll_recursive_transfer(&mut self) -> Result<()> {
        if self.active_transfer.is_some()
            || self.active_recursive_transfer.is_none()
            || self.recursive_conflict_prompt.is_some()
            || self.recursive_failure_prompt.is_some()
        {
            return Ok(());
        }

        if self
            .active_recursive_transfer
            .as_ref()
            .is_some_and(|transfer| !transfer.directories_prepared)
        {
            self.prepare_recursive_transfer_directories().await?;
        }

        let Some(transfer) = self.active_recursive_transfer.as_ref() else {
            return Ok(());
        };
        if transfer.is_complete() {
            let dir = transfer.plan.dir;
            let name = transfer
                .plan
                .source_root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| transfer.plan.destination_root.clone());
            self.active_recursive_transfer = None;
            match dir {
                TransferDir::Download => self.trigger_refresh_local = true,
                TransferDir::Upload => self.trigger_refresh_remote = true,
            }
            self.set_status(format!("Recursive transfer completed for {name}"));
            return Ok(());
        }

        self.start_next_recursive_file().await
    }

    async fn prepare_recursive_transfer_directories(&mut self) -> Result<()> {
        let Some(transfer) = self.active_recursive_transfer.as_ref() else {
            return Ok(());
        };
        let dir = transfer.plan.dir;
        let destination_root = transfer.plan.destination_root.clone();
        let directories = transfer.plan.directories.clone();

        match dir {
            TransferDir::Download => {
                std::fs::create_dir_all(&destination_root)?;
                for directory in directories {
                    std::fs::create_dir_all(
                        PathBuf::from(&destination_root).join(directory.relative_path),
                    )?;
                }
            }
            TransferDir::Upload => {
                let Some(client) = &self.sftp_client else {
                    return Ok(());
                };
                ensure_remote_directory(client, &destination_root).await?;
                for directory in directories {
                    let remote_path =
                        append_remote_relative_path(&destination_root, &directory.relative_path);
                    ensure_remote_directory(client, &remote_path).await?;
                }
            }
        }

        if let Some(transfer) = self.active_recursive_transfer.as_mut() {
            transfer.directories_prepared = true;
        }
        Ok(())
    }

    async fn start_next_recursive_file(&mut self) -> Result<()> {
        let Some(transfer) = self.active_recursive_transfer.as_ref() else {
            return Ok(());
        };
        let Some(file) = transfer.pending_file().cloned() else {
            return Ok(());
        };
        let dir = transfer.plan.dir;
        let source_root = transfer.plan.source_root.clone();
        let destination_root = transfer.plan.destination_root.clone();
        let current_file_index = transfer.current_file_position();
        let total_files = transfer.progress.total_files.max(1);
        let file_name = file
            .relative_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.relative_path.to_string_lossy().into_owned());

        if self
            .pending_recursive_file_has_conflict(dir, &destination_root, &file.relative_path)
            .await?
        {
            if let Some(choice) = self.conflict_resolution.default_file_conflict {
                if matches!(choice, FileConflictChoice::Skip) {
                    if let Some(transfer) = self.active_recursive_transfer.as_mut() {
                        transfer.skip_pending_file();
                    }
                    return Ok(());
                }
            } else {
                self.recursive_conflict_prompt = Some(RecursiveConflictPrompt::new(file_name));
                return Ok(());
            }
        }

        if let Some(transfer) = self.active_recursive_transfer.as_mut() {
            transfer.mark_file_pending(&file_name, file.size);
        }

        let session = self.open_transfer_session().await?;
        let (verb, handle) = match dir {
            TransferDir::Download => {
                let remote_src = append_remote_relative_path(
                    &source_root.to_string_lossy(),
                    &file.relative_path,
                );
                let local_dest = PathBuf::from(&destination_root).join(&file.relative_path);
                (
                    "Downloading",
                    download(session, remote_src, local_dest, file.size),
                )
            }
            TransferDir::Upload => {
                let local_src = source_root.join(&file.relative_path);
                let remote_dest =
                    append_remote_relative_path(&destination_root, &file.relative_path);
                ("Uploading", upload(session, local_src, remote_dest)?)
            }
        };

        let init_prog = TransferProgress {
            filename: file_name,
            total_bytes: file.size,
            transferred_bytes: 0,
            state: TransferState::InProgress,
            current_file_index,
            total_files,
        };
        self.active_transfer = Some(ActiveTransfer {
            verb,
            dir,
            progress: init_prog,
            rx: handle.rx,
            cancel: handle.cancel,
            done_at: None,
            needs_refresh: false,
        });
        Ok(())
    }

    fn handle_recursive_transfer_result(&mut self, progress: TransferProgress) {
        let Some(transfer) = self.active_recursive_transfer.as_mut() else {
            return;
        };
        transfer.update_current_file_bytes(progress.transferred_bytes);

        match progress.state {
            TransferState::Completed => {
                transfer.finish_pending_file();
            }
            TransferState::Failed(error) => {
                self.recursive_failure_prompt =
                    Some(RecursiveFailureState::for_file(&progress.filename));
                self.set_status(format!(
                    "Recursive transfer paused on {}: {error}",
                    progress.filename
                ));
            }
            TransferState::Cancelled => {
                let dir = transfer.plan.dir;
                self.active_recursive_transfer = None;
                match dir {
                    TransferDir::Download => self.trigger_refresh_local = true,
                    TransferDir::Upload => self.trigger_refresh_remote = true,
                }
                self.set_status("Recursive transfer cancelled".into());
            }
            TransferState::InProgress => {}
        }
    }

    async fn pending_recursive_file_has_conflict(
        &self,
        dir: TransferDir,
        destination_root: &str,
        relative_path: &Path,
    ) -> Result<bool> {
        match dir {
            TransferDir::Download => {
                Ok(PathBuf::from(destination_root).join(relative_path).exists())
            }
            TransferDir::Upload => {
                let Some(client) = &self.sftp_client else {
                    return Ok(false);
                };
                let remote_path = append_remote_relative_path(destination_root, relative_path);
                Ok(client.session.metadata(remote_path).await.is_ok())
            }
        }
    }

    async fn open_transfer_session(&self) -> Result<russh_sftp::client::SftpSession> {
        let Some(session) = &self.active_session else {
            bail!("missing active SSH session for transfer");
        };
        let ch = session.handle.channel_open_session().await?;
        ch.request_subsystem(true, "sftp").await?;
        Ok(russh_sftp::client::SftpSession::new(ch.into_stream()).await?)
    }

    fn queued_single_file_transfer(
        &self,
        dir: TransferDir,
        display_name: String,
        local_path: PathBuf,
        remote_path: String,
        total_bytes: u64,
    ) -> QueuedTransfer {
        QueuedTransfer::SingleFile {
            dir,
            display_name,
            local_path,
            remote_path,
            total_bytes,
        }
    }

    fn queued_recursive_transfer(
        &self,
        display_name: String,
        plan: RecursiveTransferPlan,
    ) -> QueuedTransfer {
        QueuedTransfer::Recursive { display_name, plan }
    }

    async fn start_download(&mut self) -> Result<()> {
        let queued = {
            let Some(pane) = &self.sftp_pane else {
                return Ok(());
            };
            if pane.side != PaneSide::Remote || pane.remote_selection.is_empty() {
                None
            } else {
                let selected_entries = pane
                    .remote_selection
                    .iter()
                    .filter_map(|index| pane.remote_entries.get(*index).cloned())
                    .collect::<Vec<_>>();
                let selected_count = selected_entries.len();
                let mut queued = Vec::new();
                for entry in selected_entries {
                    if entry.is_dir {
                        let Some(client) = &self.sftp_client else {
                            continue;
                        };
                        let remote_src =
                            format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name);
                        let plan =
                            build_remote_recursive_plan(client, &remote_src, &pane.local_path)
                                .await?;
                        queued.push(self.queued_recursive_transfer(entry.name.clone(), plan));
                    } else {
                        queued.push(self.queued_single_file_transfer(
                            TransferDir::Download,
                            entry.name.clone(),
                            pane.local_path.join(&entry.name),
                            format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name),
                            entry.size,
                        ));
                    }
                }
                Some((selected_count, queued))
            }
        };
        if let Some((selected_count, transfers)) = queued {
            if let Some(pane) = &mut self.sftp_pane {
                pane.clear_active_selection();
            }
            self.enqueue_transfers(transfers).await?;
            self.set_status(format!("Queued batch download for {selected_count} items"));
            return Ok(());
        }

        let Some(pane) = &self.sftp_pane else {
            return Ok(());
        };
        if pane.side != PaneSide::Remote {
            return Ok(());
        }
        let idx = pane.remote_list_state.selected().unwrap_or(0);
        let Some(entry) = pane.remote_entries.get(idx).cloned() else {
            return Ok(());
        };
        if entry.is_dir {
            let Some(client) = &self.sftp_client else {
                return Ok(());
            };
            let remote_src = format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name);
            let plan = build_remote_recursive_plan(client, &remote_src, &pane.local_path).await?;
            self.enqueue_transfers(vec![
                self.queued_recursive_transfer(entry.name.clone(), plan),
            ])
            .await?;
            self.set_status(format!("Queued recursive download for {}", entry.name));
            return Ok(());
        }

        self.enqueue_transfers(vec![self.queued_single_file_transfer(
            TransferDir::Download,
            entry.name.clone(),
            pane.local_path.join(&entry.name),
            format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name),
            entry.size,
        )])
        .await?;
        Ok(())
    }

    async fn start_upload(&mut self) -> Result<()> {
        let queued = {
            let Some(pane) = &self.sftp_pane else {
                return Ok(());
            };
            if pane.side != PaneSide::Local || pane.local_selection.is_empty() {
                None
            } else {
                let selected_entries = pane
                    .local_selection
                    .iter()
                    .filter_map(|index| pane.local_entries.get(*index).cloned())
                    .collect::<Vec<_>>();
                let selected_count = selected_entries.len();
                let mut queued = Vec::new();
                for entry in selected_entries {
                    if entry.is_dir {
                        let local_src = pane.local_path.join(&entry.name);
                        let plan = build_local_recursive_plan(&local_src, &pane.remote_path)?;
                        queued.push(self.queued_recursive_transfer(entry.name.clone(), plan));
                    } else {
                        queued.push(self.queued_single_file_transfer(
                            TransferDir::Upload,
                            entry.name.clone(),
                            pane.local_path.join(&entry.name),
                            format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name),
                            entry.size,
                        ));
                    }
                }
                Some((selected_count, queued))
            }
        };
        if let Some((selected_count, transfers)) = queued {
            if let Some(pane) = &mut self.sftp_pane {
                pane.clear_active_selection();
            }
            self.enqueue_transfers(transfers).await?;
            self.set_status(format!("Queued batch upload for {selected_count} items"));
            return Ok(());
        }

        let Some(pane) = &self.sftp_pane else {
            return Ok(());
        };
        if pane.side != PaneSide::Local {
            return Ok(());
        }
        let idx = pane.local_list_state.selected().unwrap_or(0);
        let Some(entry) = pane.local_entries.get(idx).cloned() else {
            return Ok(());
        };
        if entry.is_dir {
            let local_src = pane.local_path.join(&entry.name);
            let plan = build_local_recursive_plan(&local_src, &pane.remote_path)?;
            self.enqueue_transfers(vec![
                self.queued_recursive_transfer(entry.name.clone(), plan),
            ])
            .await?;
            self.set_status(format!("Queued recursive upload for {}", entry.name));
            return Ok(());
        }

        self.enqueue_transfers(vec![self.queued_single_file_transfer(
            TransferDir::Upload,
            entry.name.clone(),
            pane.local_path.join(&entry.name),
            format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name),
            entry.size,
        )])
        .await?;
        Ok(())
    }

    async fn start_sftp_delete(&mut self) -> Result<()> {
        let Some(pane) = &self.sftp_pane else {
            return Ok(());
        };

        match pane.side {
            PaneSide::Local => {
                let selected_entries = pane
                    .local_selection
                    .iter()
                    .filter_map(|index| pane.local_entries.get(*index).cloned())
                    .collect::<Vec<_>>();

                for entry in &selected_entries {
                    let path = pane.local_path.join(&entry.name);
                    if entry.is_dir {
                        std::fs::remove_dir_all(&path)?;
                    } else {
                        std::fs::remove_file(&path)?;
                    }
                }

                if let Some(pane) = &mut self.sftp_pane {
                    pane.clear_active_selection();
                }
                self.trigger_refresh_local = true;
                self.set_status(format!("Deleted {} local item(s)", selected_entries.len()));
            }
            PaneSide::Remote => {
                let selected_entries = pane
                    .remote_selection
                    .iter()
                    .filter_map(|index| pane.remote_entries.get(*index).cloned())
                    .collect::<Vec<_>>();

                let Some(client) = &self.sftp_client else {
                    bail!("missing SFTP client for remote delete");
                };

                for entry in &selected_entries {
                    let remote_path =
                        format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name);
                    client
                        .remove_path_recursive(&remote_path, entry.is_dir)
                        .await?;
                }

                if let Some(pane) = &mut self.sftp_pane {
                    pane.clear_active_selection();
                }
                self.trigger_refresh_remote = true;
                self.set_status(format!("Deleted {} remote item(s)", selected_entries.len()));
            }
        }

        Ok(())
    }

    async fn finish_ssh_connect(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        host: &Host,
        session: ActiveSession,
    ) -> Result<()> {
        let mut session = self
            .authenticate_connected_session(terminal, bus, host, session)
            .await?;
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        // Reserve 1 row for the status bar.
        let term_rows = rows.saturating_sub(3).max(1);
        session.request_pty(cols, term_rows).await?;

        self.terminal_emulator = Some(TerminalEmulator::new(cols, term_rows));
        self.active_session = Some(session);
        self.current_host_alias = Some(host.alias.clone());
        self.mode = AppMode::Ssh;
        self.connection_history.record(&host.alias);
        self.ssh_last_size = Some((cols, rows));
        Ok(())
    }

    async fn authenticate_connected_session(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        host: &Host,
        mut session: ActiveSession,
    ) -> Result<ActiveSession> {
        #[cfg(unix)]
        if auth::try_agent_auth(&mut session.handle, &host.user)
            .await
            .unwrap_or(false)
        {
            return Ok(session);
        }

        for key_path in &host.identity_files {
            let expanded = expand_tilde(key_path);
            if try_key_auth(&mut session.handle, &host.user, &expanded, None)
                .await
                .unwrap_or(false)
            {
                return Ok(session);
            }

            let identity_hint = key_path.display().to_string();
            let key_passphrase_key = SecretKey::new(
                &host.id,
                SecretKind::KeyPassphrase,
                Some(identity_hint.as_str()),
            );
            if let Ok(Some(pass)) = self.secret_store.get(&key_passphrase_key)
                && try_key_auth(&mut session.handle, &host.user, &expanded, Some(&pass))
                    .await
                    .unwrap_or(false)
            {
                return Ok(session);
            }

            let title = self
                .key_passphrase_prompt_title(identity_hint.as_str(), &key_passphrase_key.account);
            if let Some(pass) = self.prompt_password(terminal, bus, &title).await?
                && try_key_auth(&mut session.handle, &host.user, &expanded, Some(&pass))
                    .await
                    .unwrap_or(false)
            {
                match self.secret_store.set(&key_passphrase_key, &pass) {
                    Ok(()) => self.clear_secret_save_failure(&key_passphrase_key.account),
                    Err(error) => {
                        self.record_secret_save_failure(key_passphrase_key.account.clone(), &error)
                    }
                }
                return Ok(session);
            }
        }

        let password_key = SecretKey::new(&host.id, SecretKind::LoginPassword, None);
        if let Ok(Some(pass)) = self.secret_store.get(&password_key) {
            let ok = session
                .handle
                .authenticate_password(&host.user, &pass)
                .await?
                .success();
            if ok {
                return Ok(session);
            }
        }

        let title = self.password_prompt_title(&host.user, &host.hostname, &password_key.account);
        if let Some(pass) = self.prompt_password(terminal, bus, &title).await? {
            let ok = session
                .handle
                .authenticate_password(&host.user, &pass)
                .await?
                .success();
            if ok {
                match self.secret_store.set(&password_key, &pass) {
                    Ok(()) => self.clear_secret_save_failure(&password_key.account),
                    Err(error) => {
                        self.record_secret_save_failure(password_key.account.clone(), &error)
                    }
                }
                return Ok(session);
            }
        }

        bail!("all authentication methods failed")
    }

    async fn prompt_password(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        title: &str,
    ) -> Result<Option<String>> {
        self.pwd_dialog = Some(PwdDialog {
            dialog: PasswordDialog::new(title),
            result: None,
        });

        loop {
            self.render(terminal)?;
            match bus.next().await {
                Some(AppEvent::Input(data)) => {
                    for key in decode_tui_keys(&data) {
                        self.handle_pwd_key(key);
                    }
                }
                Some(AppEvent::Tick) => {}
                None => {
                    self.pwd_dialog = None;
                    return Ok(None);
                }
            }

            if let Some(result) = self.pwd_dialog.as_mut().and_then(|pwd| pwd.result.take()) {
                self.pwd_dialog = None;
                return Ok(result);
            }
        }
    }

    async fn switch_ssh_to_sftp(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        if self.sftp_client.is_none() || self.sftp_pane.is_none() {
            let Some(session) = self.active_session.as_ref() else {
                bail!("SSH session does not exist");
            };
            let client = SftpClient::open(session).await?;
            let home = client.home_dir().await;
            let remote_entries = client.list_dir(&home).await.unwrap_or_default();
            let local_path = std::env::current_dir().unwrap_or_default();
            let local_entries = list_local(&local_path).unwrap_or_default();

            let mut pane = SftpPaneState::new(home);
            pane.remote_entries = remote_entries;
            pane.local_entries = local_entries;
            self.sftp_client = Some(client);
            self.sftp_pane = Some(pane);
        }

        terminal.clear()?;
        self.mode = AppMode::Sftp;
        Ok(())
    }

    async fn resume_ssh_from_sftp(&mut self) -> Result<()> {
        let Some(session) = self.active_session.as_mut() else {
            self.clear_connection_state();
            self.mode = AppMode::Main;
            return Ok(());
        };

        if !session.has_pty() {
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
            let term_rows = rows.saturating_sub(3).max(1);
            session.request_pty(cols, term_rows).await?;
            self.terminal_emulator = Some(TerminalEmulator::new(cols, term_rows));
            self.ssh_last_size = Some((cols, rows));
        }

        self.mode = AppMode::Ssh;
        Ok(())
    }

    async fn handle_ssh_input(&mut self, data: Vec<u8>) -> Result<()> {
        let (forward, switch) = split_ssh_input(&data, SWITCH_SEQ);
        if !forward.is_empty()
            && let Some(session) = self.active_session.as_mut()
        {
            session.write_input(&forward).await?;
        }
        if switch {
            self.trigger_ssh_to_sftp = true;
        }
        Ok(())
    }

    async fn handle_ssh_channel_msg(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        msg: Option<ChannelMsg>,
    ) -> Result<()> {
        match msg {
            Some(ChannelMsg::Data { ref data }) => {
                if let Some(emulator) = &mut self.terminal_emulator {
                    emulator.process(data);
                }
            }
            Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                if let Some(emulator) = &mut self.terminal_emulator {
                    emulator.process(data);
                }
            }
            Some(ChannelMsg::ExitStatus { .. }) | Some(ChannelMsg::Eof) | None => {
                self.leave_ssh_mode(terminal).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn leave_ssh_mode(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        terminal.clear()?;
        self.ssh_last_size = None;
        self.terminal_emulator = None;
        if let Some(session) = self.active_session.take() {
            let _ = session.disconnect().await;
        }
        self.clear_connection_state();
        self.mode = AppMode::Main;
        Ok(())
    }

    async fn sync_ssh_size(&mut self) -> Result<()> {
        let Some(session) = self.active_session.as_mut() else {
            return Ok(());
        };
        if !session.has_pty() {
            return Ok(());
        }
        let size = crossterm::terminal::size().unwrap_or((80, 24));
        if self.ssh_last_size != Some(size) {
            let term_rows = size.1.saturating_sub(3).max(1);
            session.resize_pty(size.0, term_rows).await?;
            if let Some(emulator) = &mut self.terminal_emulator {
                emulator.resize(size.0, term_rows);
            }
            self.ssh_last_size = Some(size);
        }
        Ok(())
    }

    fn render(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        let mut list_state = std::mem::take(&mut self.list_state);
        terminal.draw(|f| match self.mode {
            AppMode::Main => {
                main_view::render(f, self, &mut list_state);
                if let Some(pwd) = &self.pwd_dialog {
                    pwd.dialog.render(f);
                }
                if self.confirm_delete
                    && let Some(&idx) = self
                        .filtered_indices
                        .get(list_state.selected().unwrap_or(0))
                {
                    let alias = self.hosts[idx].alias.clone();
                    ConfirmDialog::new(
                        "Delete Host",
                        &format!("Delete \"{alias}\"? (sush config only)"),
                    )
                    .render(f);
                }
                if self.show_import_prompt {
                    ConfirmDialog::new(
                        "Import SSH Config",
                        "Found hosts in ~/.ssh/config. Import them?",
                    )
                    .render(f);
                }
            }
            AppMode::Sftp => {
                let status_msg = self.remote_edit_status().map(str::to_owned);
                let transfer_badge = self.global_transfer_badge();
                if let Some(pane) = &mut self.sftp_pane {
                    let alias = self.current_host_alias.as_deref().unwrap_or("");
                    sftp_view::render(
                        f,
                        alias,
                        pane,
                        status_msg.as_deref(),
                        transfer_badge.as_ref(),
                    );
                }
                if let Some(pwd) = &self.pwd_dialog {
                    pwd.dialog.render(f);
                }
                if let Some(prompt) = &self.recursive_conflict_prompt {
                    let apply_label = if prompt.apply_to_remaining {
                        "on"
                    } else {
                        "off"
                    };
                    ChoiceDialog {
                        title: "File Conflict",
                        message: &format!(
                            "Conflict on {}. Toggle apply-to-remaining: {apply_label}",
                            prompt.file_name
                        ),
                        hints: vec![
                            ("o", "Overwrite"),
                            ("s", "Skip"),
                            ("a", "Apply"),
                            ("Esc", "Cancel"),
                        ],
                    }
                    .render(f);
                }
                if let Some(prompt) = &self.recursive_failure_prompt {
                    ChoiceDialog {
                        title: "Transfer Failed",
                        message: &format!(
                            "File {} failed during recursive transfer",
                            prompt.file_name
                        ),
                        hints: vec![("r", "Retry"), ("s", "Skip"), ("c/Esc", "Cancel")],
                    }
                    .render(f);
                }
                if let Some(confirm) = self.sftp_delete_confirm {
                    let scope = match confirm.side {
                        PaneSide::Local => "local",
                        PaneSide::Remote => "remote",
                    };
                    ChoiceDialog {
                        title: "Delete Selected",
                        message: &format!(
                            "Delete {} selected {scope} item(s)?",
                            confirm.selected_count
                        ),
                        hints: vec![("y", "Delete"), ("n/Esc", "Cancel")],
                    }
                    .render(f);
                }
            }
            AppMode::Ssh => {
                let transfer_badge = self.global_transfer_badge();
                if let Some(emulator) = &self.terminal_emulator {
                    let alias = self.current_host_alias.as_deref().unwrap_or("");
                    ssh_view::render(f, alias, emulator, transfer_badge.as_ref());
                }
            }
            AppMode::Edit => {
                if let Some(draft) = &self.edit_draft {
                    let all_tags: Vec<String> = {
                        let mut t: Vec<String> = self
                            .hosts
                            .iter()
                            .flat_map(|h| h.tags.iter().cloned())
                            .collect();
                        t.sort();
                        t.dedup();
                        t
                    };
                    let proxy_jump_aliases: Vec<String> =
                        self.hosts.iter().map(|host| host.alias.clone()).collect();
                    edit_view::render(f, draft, &all_tags, &proxy_jump_aliases);
                }
            }
            AppMode::ImportSshConfig => {
                if let Some(state) = &self.import_state {
                    import_view::render(f, state);
                }
            }
            AppMode::ForwardingManager => {
                if let Some(state) = &mut self.forwarding_state {
                    crate::tui::views::forwarding_view::render(f, state, &self.hosts);
                }
                if let Some(edit) = &self.forward_edit {
                    crate::tui::views::forward_edit::render(f, edit);
                }
            }
            AppMode::FolderView => {
                if let Some(fv) = &self.folder_view_state {
                    let probe: Option<Option<bool>> = if self.probe_result.is_some() {
                        Some(self.probe_result)
                    } else if self.probe_rx.is_some() {
                        Some(None)
                    } else {
                        None
                    };
                    folder_view::render(f, fv, &self.hosts, &self.folder_host_indices, probe);
                }
            }
        })?;
        self.list_state = list_state;
        Ok(())
    }
}

fn start_remote_edit_watcher(
    path: &Path,
) -> Result<(RecommendedWatcher, tokio::sync::mpsc::UnboundedReceiver<()>)> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if result.is_ok() {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(path, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}

fn append_remote_relative_path(base: &str, relative: &Path) -> String {
    let child = relative.to_string_lossy().replace('\\', "/");
    match base {
        "/" => format!("/{child}"),
        _ if base.ends_with('/') => format!("{base}{child}"),
        _ => format!("{base}/{child}"),
    }
}

async fn ensure_remote_directory(client: &SftpClient, path: &str) -> Result<()> {
    if client.session.metadata(path).await.is_ok() {
        return Ok(());
    }

    match client.session.create_dir(path).await {
        Ok(()) => Ok(()),
        Err(error) => {
            if client.session.metadata(path).await.is_ok() {
                Ok(())
            } else {
                Err(Into::into(error))
            }
        }
    }
}

fn fingerprint_file(path: &Path) -> Result<String> {
    let data = std::fs::read(path)?;
    let metadata = std::fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .unwrap_or(Duration::ZERO)
        .as_millis();
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    Ok(format!("{}:{modified}:{}", data.len(), hasher.finish()))
}

const SWITCH_SEQ: &[u8] = b"\x1c";

fn is_switch_key(k: KeyEvent) -> bool {
    matches!(
        (k.code, k.modifiers),
        (KeyCode::Char('\\'), KeyModifiers::CONTROL)
    )
}

fn split_ssh_input(data: &[u8], switch_seq: &[u8]) -> (Vec<u8>, bool) {
    if switch_seq.is_empty() {
        return (data.to_vec(), false);
    }

    if let Some(pos) = data
        .windows(switch_seq.len())
        .position(|window| window == switch_seq)
    {
        (data[..pos].to_vec(), true)
    } else {
        (data.to_vec(), false)
    }
}

fn decode_tui_keys(data: &[u8]) -> Vec<KeyEvent> {
    let mut keys = Vec::new();
    let mut i = 0;

    while i < data.len() {
        if let Some((key, consumed)) = decode_escape_key(&data[i..]) {
            keys.push(key);
            i += consumed;
            continue;
        }

        let byte = data[i];
        match byte {
            b'\r' | b'\n' => {
                keys.push(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                i += 1;
            }
            b'\t' => {
                keys.push(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                i += 1;
            }
            0x03 => {
                keys.push(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
                i += 1;
            }
            0x1c => {
                keys.push(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
                i += 1;
            }
            0x08 | 0x7f => {
                keys.push(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
                i += 1;
            }
            0x01..=0x1a => {
                let ch = ((byte - 1) + b'a') as char;
                keys.push(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL));
                i += 1;
            }
            0x1b => {
                keys.push(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                i += 1;
            }
            _ if byte.is_ascii() => {
                keys.push(KeyEvent::new(
                    KeyCode::Char(byte as char),
                    KeyModifiers::NONE,
                ));
                i += 1;
            }
            _ => {
                let width = utf8_char_width(byte).min(data.len() - i);
                if let Ok(text) = std::str::from_utf8(&data[i..i + width])
                    && let Some(ch) = text.chars().next()
                {
                    keys.push(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
                    i += ch.len_utf8();
                    continue;
                }
                i += 1;
            }
        }
    }

    keys
}

fn decode_escape_key(data: &[u8]) -> Option<(KeyEvent, usize)> {
    let patterns = [
        (b"\x1b[A".as_slice(), KeyCode::Up),
        (b"\x1b[B".as_slice(), KeyCode::Down),
        (b"\x1b[C".as_slice(), KeyCode::Right),
        (b"\x1b[D".as_slice(), KeyCode::Left),
        (b"\x1b[H".as_slice(), KeyCode::Home),
        (b"\x1b[F".as_slice(), KeyCode::End),
        (b"\x1b[3~".as_slice(), KeyCode::Delete),
    ];

    patterns.iter().find_map(|(pattern, code)| {
        data.starts_with(pattern)
            .then(|| (KeyEvent::new(*code, KeyModifiers::NONE), pattern.len()))
    })
}

fn utf8_char_width(byte: u8) -> usize {
    match byte {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

fn expand_tilde(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    p.to_path_buf()
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut out = stdout();
    crossterm::execute!(out, crossterm::terminal::EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::history::ConnectionHistory;
    use crate::config::host::{Host, HostSource};
    use crate::config::secrets::{FakeBackend, SecretStore};
    use crate::config::store::{Metadata, SecretSaveFailure};

    fn app_with(hosts: Vec<Host>) -> App {
        let test_config_dir = tempfile::TempDir::new().unwrap();
        App {
            mode: AppMode::Main,
            hosts,
            search_query: String::new(),
            filtered_indices: vec![],
            should_quit: false,
            main_focus: MainFocus::HostList,
            main_focus_before_search: MainFocus::HostList,
            show_folder_sidebar: false,
            list_state: ListState::default(),
            trigger_connect: false,
            trigger_sftp: false,
            trigger_pane_enter: false,
            trigger_download: false,
            trigger_upload: false,
            trigger_sftp_delete: false,
            trigger_remote_edit: false,
            trigger_refresh_local: false,
            trigger_refresh_remote: false,
            trigger_ssh_resume: false,
            trigger_ssh_to_sftp: false,
            active_session: None,
            sftp_client: None,
            sftp_pane: None,
            current_host_alias: None,
            active_transfer: None,
            queued_transfers: VecDeque::new(),
            queue_completed_count: 0,
            queue_total_count: 0,
            active_recursive_transfer: None,
            conflict_resolution: ConflictResolutionState::default(),
            recursive_conflict_prompt: None,
            recursive_failure_prompt: None,
            remote_edit_session: None,
            last_ctrl_c: None,
            ssh_last_size: None,
            terminal_emulator: None,
            edit_draft: None,
            import_state: None,
            confirm_delete: false,
            sftp_delete_confirm: None,
            import_prompted: false,
            metadata: Metadata::default(),
            connection_history: ConnectionHistory::load(std::path::PathBuf::from(
                "/tmp/sush-test-history.toml",
            )),
            hover_host_id: None,
            hover_since: None,
            probe_rx: None,
            probe_result: None,
            status_msg: None,
            pending_connection: None,
            show_import_prompt: false,
            folder_view_state: None,
            folder_host_indices: vec![],
            forwarding_state: None,
            forward_edit: None,
            secret_store: SecretStore::new(Box::new(FakeBackend::available())),
            pwd_dialog: None,
            test_config_dir: Some(test_config_dir),
        }
    }

    fn app_with_metadata_failures() -> App {
        let mut app = app_with(vec![]);
        app.metadata = Metadata {
            ssh_config_hash: String::new(),
            import_prompted: false,
            secret_save_failures: vec![SecretSaveFailure {
                account: "host-1:login_password".into(),
                reason: "system keyring is unavailable".into(),
            }],
        };
        app.secret_store = SecretStore::new(Box::new(FakeBackend::available()));
        app
    }

    fn mk(id: &str) -> Host {
        Host {
            id: id.into(),
            alias: id.into(),
            hostname: "1".into(),
            port: 22,
            user: "u".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: vec![],
            description: String::new(),
            source: HostSource::Manual,
            forwards: vec![],
        }
    }

    #[test]
    fn resolve_connection_route_uses_direct_target_without_proxy_jump() {
        let target = mk("ubuntu2");
        let app = app_with(vec![target.clone()]);

        match app.resolve_connection_route(&target).unwrap() {
            ConnectionRoute::Direct(host) => assert_eq!(host.alias, "ubuntu2"),
            ConnectionRoute::ViaProxyJump { .. } => panic!("expected direct route"),
        }
    }

    #[test]
    fn resolve_connection_route_uses_bastion_for_proxy_jump() {
        let bastion = mk("ubuntu1");
        let mut target = mk("ubuntu2");
        target.proxy_jump = Some("ubuntu1".into());
        let app = app_with(vec![bastion.clone(), target.clone()]);

        match app.resolve_connection_route(&target).unwrap() {
            ConnectionRoute::ViaProxyJump {
                bastion: resolved_bastion,
                target: resolved_target,
            } => {
                assert_eq!(resolved_bastion.alias, "ubuntu1");
                assert_eq!(resolved_target.alias, "ubuntu2");
            }
            ConnectionRoute::Direct(_) => panic!("expected proxy jump route"),
        }
    }

    #[test]
    fn resolve_connection_route_rejects_missing_proxy_jump_host() {
        let mut target = mk("ubuntu2");
        target.proxy_jump = Some("missing".into());
        let app = app_with(vec![target.clone()]);

        let error = app
            .resolve_connection_route(&target)
            .unwrap_err()
            .to_string();

        assert!(error.contains("proxy jump host 'missing' not found"));
    }

    fn app_with_sftp_pane(side: PaneSide, entries: Vec<crate::sftp::client::FileEntry>) -> App {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = side;
        match side {
            PaneSide::Local => pane.local_entries = entries,
            PaneSide::Remote => pane.remote_entries = entries,
        }
        match side {
            PaneSide::Local => pane.local_list_state.select(Some(0)),
            PaneSide::Remote => pane.remote_list_state.select(Some(0)),
        }
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;
        app
    }

    fn file_entry(name: &str, is_dir: bool) -> crate::sftp::client::FileEntry {
        crate::sftp::client::FileEntry {
            name: name.into(),
            is_dir,
            size: 0,
        }
    }

    fn active_transfer_for_test(
        dir: TransferDir,
        transferred_bytes: u64,
        total_bytes: u64,
    ) -> ActiveTransfer {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        ActiveTransfer {
            verb: match dir {
                TransferDir::Download => "Downloading",
                TransferDir::Upload => "Uploading",
            },
            dir,
            progress: TransferProgress {
                filename: "file.txt".into(),
                total_bytes,
                transferred_bytes,
                state: TransferState::InProgress,
                current_file_index: 1,
                total_files: 1,
            },
            rx,
            cancel: tokio_util::sync::CancellationToken::new(),
            done_at: None,
            needs_refresh: false,
        }
    }

    fn completed_transfer_for_test(dir: TransferDir) -> ActiveTransfer {
        let mut transfer = active_transfer_for_test(dir, 100, 100);
        transfer.progress.state = TransferState::Completed;
        transfer.done_at = Some(Instant::now() - Duration::from_secs(4));
        transfer
    }

    fn queued_file_upload_for_test(name: &str) -> QueuedTransfer {
        QueuedTransfer::SingleFile {
            dir: TransferDir::Upload,
            display_name: name.into(),
            local_path: PathBuf::from(format!("/tmp/{name}")),
            remote_path: format!("/remote/{name}"),
            total_bytes: 1,
        }
    }

    fn queued_file_download_for_test(name: &str) -> QueuedTransfer {
        QueuedTransfer::SingleFile {
            dir: TransferDir::Download,
            display_name: name.into(),
            local_path: PathBuf::from(format!("/tmp/{name}")),
            remote_path: format!("/remote/{name}"),
            total_bytes: 1,
        }
    }

    fn queued_recursive_upload_for_test(name: &str) -> QueuedTransfer {
        QueuedTransfer::Recursive {
            display_name: name.into(),
            plan: RecursiveTransferPlan {
                dir: TransferDir::Upload,
                source_root: PathBuf::from(format!("/tmp/{name}")),
                destination_root: format!("/remote/{name}"),
                directories: vec![],
                files: vec![],
            },
        }
    }

    #[test]
    fn forwarding_view_state_lists_hosts_without_rules_for_creation() {
        let mut host_without_rules = mk("a");
        let mut host_with_rules = mk("b");
        host_with_rules
            .forwards
            .push(crate::config::host::ForwardRule {
                id: "fwd-1".into(),
                name: "web".into(),
                kind: crate::config::host::ForwardKind::Local,
                local_port: 8080,
                remote_host: Some("localhost".into()),
                remote_port: Some(80),
                auto_start: false,
            });
        host_without_rules.forwards.clear();

        let state = ForwardingViewState::new(&[host_without_rules, host_with_rules]);

        assert_eq!(state.host_indices, vec![0, 1]);
        assert_eq!(state.host_list_state.selected(), Some(0));
        assert_eq!(state.selected_host_idx(), Some(0));
    }

    #[test]
    fn p_key_opens_forwarding_manager() {
        let mut app = app_with(vec![mk("web")]);

        app.handle_main_key_hostlist(KeyEvent::from(KeyCode::Char('p')));

        assert_eq!(app.mode, AppMode::ForwardingManager);
        assert!(app.forwarding_state.is_some());
    }

    #[test]
    fn n_key_opens_forward_creation_for_host_without_existing_rules() {
        let mut app = app_with(vec![mk("web")]);

        app.handle_main_key_hostlist(KeyEvent::from(KeyCode::Char('p')));
        app.handle_forwarding_key(KeyEvent::from(KeyCode::Char('n')));

        let edit = app.forward_edit.as_ref().expect("forward edit should open");
        assert_eq!(edit.host_id, "web");
        assert_eq!(edit.host_alias, "web");
    }

    #[test]
    fn saving_new_forward_keeps_selected_host() {
        let mut app = app_with(vec![mk("a"), mk("b")]);

        app.handle_main_key_hostlist(KeyEvent::from(KeyCode::Char('p')));
        app.forwarding_state
            .as_mut()
            .expect("forwarding state should exist")
            .host_list_state
            .select(Some(1));

        app.handle_forwarding_key(KeyEvent::from(KeyCode::Char('n')));

        let edit = app
            .forward_edit
            .as_mut()
            .expect("forward edit should open for selected host");
        edit.name = "web".into();
        edit.local_port = "8080".into();
        edit.remote_host = "localhost".into();
        edit.remote_port = "80".into();

        app.handle_forward_edit_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        let state = app
            .forwarding_state
            .as_ref()
            .expect("forwarding state should still exist");
        assert_eq!(state.selected_host_idx(), Some(1));
        assert_eq!(app.hosts[1].forwards.len(), 1);
    }

    #[test]
    fn search_filters_hosts() {
        let mut app = app_with(vec![mk("web"), mk("db")]);
        app.search_query = "web".into();
        app.apply_search();
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.hosts[app.filtered_indices[0]].alias, "web");
    }

    #[test]
    fn empty_search_shows_all() {
        let mut app = app_with(vec![mk("a"), mk("b"), mk("c")]);
        app.apply_search();
        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
    }

    #[test]
    fn decode_switch_key_from_raw_input() {
        let keys = decode_tui_keys(b"\x1c");
        assert_eq!(keys.len(), 1);
        assert!(is_switch_key(keys[0]));
    }

    #[test]
    fn split_ssh_input_stops_before_switch_key() {
        let (forward, switched) = split_ssh_input(b"ls\x1c", SWITCH_SEQ);
        assert_eq!(forward, b"ls");
        assert!(switched);
    }

    #[test]
    fn app_test_helper_carries_secret_failure_metadata() {
        let app = app_with_metadata_failures();
        assert_eq!(app.metadata.secret_save_failures.len(), 1);
        assert_eq!(
            app.metadata.secret_save_failures[0].account,
            "host-1:login_password"
        );
    }

    #[test]
    fn password_prompt_title_includes_previous_save_failure_reason() {
        let mut app = app_with_metadata_failures();
        let title =
            app.password_prompt_title("deploy", "prod.example.com", "host-1:login_password");
        assert!(title.contains("Password was not saved last time"));
        assert!(title.contains("system keyring is unavailable"));
        assert!(title.contains("Password for deploy@prod.example.com:"));
    }

    #[test]
    fn record_secret_save_failure_stores_user_message() {
        let mut app = app_with(vec![]);
        app.record_secret_save_failure(
            "host-1:login_password".into(),
            &crate::config::secrets::SecretError::Unavailable(
                "linux secret service not found".into(),
            ),
        );
        assert_eq!(app.metadata.secret_save_failures.len(), 1);
        assert!(
            app.metadata.secret_save_failures[0]
                .reason
                .contains("system keyring is unavailable")
        );
    }

    #[test]
    fn key_passphrase_account_uses_identity_path() {
        let key = SecretKey::new(
            "host-1",
            SecretKind::KeyPassphrase,
            Some("/Users/me/.ssh/id_ed25519"),
        );
        assert!(key.account.contains("id_ed25519"));
    }

    #[test]
    fn key_passphrase_prompt_title_includes_failure_reason() {
        let mut app = app_with(vec![]);
        app.metadata.upsert_secret_failure(
            "host-1:key_passphrase:/Users/me/.ssh/id_ed25519".into(),
            "permission denied by system keyring".into(),
        );
        let title = app.key_passphrase_prompt_title(
            "/Users/me/.ssh/id_ed25519",
            "host-1:key_passphrase:/Users/me/.ssh/id_ed25519",
        );
        assert!(title.contains("Password was not saved last time"));
        assert!(title.contains("permission denied by system keyring"));
        assert!(title.contains("Key passphrase (/Users/me/.ssh/id_ed25519):"));
    }

    #[test]
    fn remote_edit_session_starts_in_opening_state() {
        let session = RemoteEditSession::for_test(
            "/remote/app.conf".into(),
            PathBuf::from("/tmp/app.conf"),
            "hash-1".into(),
        );
        assert_eq!(session.sync_state, RemoteEditSyncState::Opening);
        assert_eq!(session.last_uploaded_fingerprint, "hash-1");
        assert_eq!(session.last_seen_fingerprint, "hash-1");
    }

    #[test]
    fn failed_upload_keeps_session_active() {
        let mut session = RemoteEditSession::for_test(
            "/remote/app.conf".into(),
            PathBuf::from("/tmp/app.conf"),
            "hash-1".into(),
        );
        session.mark_upload_failed("network error".into());
        assert_eq!(session.sync_state, RemoteEditSyncState::UploadFailed);
        assert_eq!(session.last_error.as_deref(), Some("network error"));
    }

    #[test]
    fn pressing_e_in_remote_pane_triggers_remote_edit() {
        let mut app = app_with_sftp_pane(
            PaneSide::Remote,
            vec![crate::sftp::client::FileEntry {
                name: "hosts".into(),
                is_dir: false,
                size: 12,
            }],
        );
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char('e')));
        assert!(app.trigger_remote_edit);
    }

    #[test]
    fn pressing_e_in_local_pane_does_not_trigger_remote_edit() {
        let mut app = app_with_sftp_pane(
            PaneSide::Local,
            vec![crate::sftp::client::FileEntry {
                name: "hosts".into(),
                is_dir: false,
                size: 12,
            }],
        );
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char('e')));
        assert!(!app.trigger_remote_edit);
    }

    #[test]
    fn pressing_e_on_remote_directory_does_not_trigger_remote_edit() {
        let mut app = app_with_sftp_pane(
            PaneSide::Remote,
            vec![crate::sftp::client::FileEntry {
                name: "etc".into(),
                is_dir: true,
                size: 0,
            }],
        );
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char('e')));
        assert!(!app.trigger_remote_edit);
    }

    #[test]
    fn pressing_q_with_active_remote_edit_does_not_exit_sftp() {
        let mut app = app_with_sftp_pane(
            PaneSide::Remote,
            vec![crate::sftp::client::FileEntry {
                name: "hosts".into(),
                is_dir: false,
                size: 12,
            }],
        );
        app.remote_edit_session = Some(RemoteEditSession::for_test(
            "/remote/hosts".into(),
            PathBuf::from("/tmp/hosts"),
            "hash-1".into(),
        ));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char('q')));
        assert_eq!(app.mode, AppMode::Sftp);
        assert!(app.sftp_pane.is_some());
        assert!(app.status_msg.is_some());
    }

    #[test]
    fn tab_switches_active_pane_without_resetting_local_selection() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_entries = vec![file_entry("a.txt", false), file_entry("b.txt", false)];
        pane.remote_entries = vec![file_entry("r1.txt", false), file_entry("r2.txt", false)];
        pane.local_list_state.select(Some(1));
        pane.remote_list_state.select(Some(0));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_sftp_key(KeyEvent::from(KeyCode::Tab));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Tab));

        let pane = app.sftp_pane.as_ref().unwrap();
        assert_eq!(pane.side, PaneSide::Local);
        assert_eq!(pane.local_list_state.selected(), Some(1));
    }

    #[test]
    fn switching_to_remote_preserves_remote_selection() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_entries = vec![file_entry("a.txt", false)];
        pane.remote_entries = vec![file_entry("r1.txt", false), file_entry("r2.txt", false)];
        pane.local_list_state.select(Some(0));
        pane.remote_list_state.select(Some(1));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_sftp_key(KeyEvent::from(KeyCode::Tab));

        let pane = app.sftp_pane.as_ref().unwrap();
        assert_eq!(pane.side, PaneSide::Remote);
        assert_eq!(pane.remote_list_state.selected(), Some(1));
    }

    #[test]
    fn pane_select_next_moves_only_active_pane_selection() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Remote;
        pane.local_entries = vec![file_entry("a.txt", false)];
        pane.remote_entries = vec![file_entry("r1.txt", false), file_entry("r2.txt", false)];
        pane.local_list_state.select(Some(0));
        pane.remote_list_state.select(Some(0));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.pane_select_next();

        let pane = app.sftp_pane.as_ref().unwrap();
        assert_eq!(pane.remote_list_state.selected(), Some(1));
        assert_eq!(pane.local_list_state.selected(), Some(0));
    }

    #[test]
    fn space_toggles_active_local_entry_selection() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_entries = vec![file_entry("a.txt", false)];
        pane.local_list_state.select(Some(0));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));

        let pane = app.sftp_pane.as_ref().unwrap();
        assert!(pane.local_selection.contains(&0));
        assert_eq!(pane.local_selection_anchor, Some(0));
    }

    #[test]
    fn esc_clears_active_remote_multi_select() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Remote;
        pane.remote_entries = vec![file_entry("r1.txt", false)];
        pane.remote_selection.insert(0);
        pane.remote_selection_anchor = Some(0);
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_key(KeyEvent::from(KeyCode::Esc));

        let pane = app.sftp_pane.as_ref().unwrap();
        assert!(pane.remote_selection.is_empty());
        assert_eq!(pane.remote_selection_anchor, None);
    }

    #[test]
    fn double_space_selects_downward_range_from_anchor() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Remote;
        pane.remote_entries = vec![
            file_entry("a", false),
            file_entry("b", false),
            file_entry("c", false),
        ];
        pane.remote_list_state.select(Some(0));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        app.sftp_pane
            .as_mut()
            .unwrap()
            .remote_list_state
            .select(Some(2));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));

        let pane = app.sftp_pane.as_ref().unwrap();
        assert_eq!(pane.remote_selection.len(), 3);
        assert!(pane.remote_selection.contains(&0));
        assert!(pane.remote_selection.contains(&1));
        assert!(pane.remote_selection.contains(&2));
    }

    #[test]
    fn double_space_selects_upward_range_from_anchor() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_entries = vec![
            file_entry("a", false),
            file_entry("b", false),
            file_entry("c", false),
        ];
        pane.local_list_state.select(Some(2));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        app.sftp_pane
            .as_mut()
            .unwrap()
            .local_list_state
            .select(Some(0));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));

        let pane = app.sftp_pane.as_ref().unwrap();
        assert_eq!(pane.local_selection.len(), 3);
        assert!(pane.local_selection.contains(&0));
        assert!(pane.local_selection.contains(&1));
        assert!(pane.local_selection.contains(&2));
    }

    #[test]
    fn double_space_selects_range_after_moving_slowly_from_anchor() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Remote;
        pane.remote_entries = vec![
            file_entry("a", false),
            file_entry("b", false),
            file_entry("c", false),
        ];
        pane.remote_list_state.select(Some(0));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        std::thread::sleep(std::time::Duration::from_millis(600));
        app.sftp_pane
            .as_mut()
            .unwrap()
            .remote_list_state
            .select(Some(2));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));

        let pane = app.sftp_pane.as_ref().unwrap();
        assert_eq!(pane.remote_selection.len(), 3);
        assert!(pane.remote_selection.contains(&0));
        assert!(pane.remote_selection.contains(&1));
        assert!(pane.remote_selection.contains(&2));
    }

    #[test]
    fn single_click_sets_anchor_for_next_range_without_reusing_old_anchor() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Remote;
        pane.remote_entries = vec![
            file_entry("a", false),
            file_entry("b", false),
            file_entry("c", false),
            file_entry("d", false),
            file_entry("e", false),
            file_entry("f", false),
        ];
        pane.remote_list_state.select(Some(0));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        app.sftp_pane
            .as_mut()
            .unwrap()
            .remote_list_state
            .select(Some(2));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));

        app.sftp_pane
            .as_mut()
            .unwrap()
            .remote_list_state
            .select(Some(4));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));

        app.sftp_pane
            .as_mut()
            .unwrap()
            .remote_list_state
            .select(Some(5));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));
        app.handle_sftp_key(KeyEvent::from(KeyCode::Char(' ')));

        let pane = app.sftp_pane.as_ref().unwrap();
        assert!(pane.remote_selection.contains(&0));
        assert!(pane.remote_selection.contains(&1));
        assert!(pane.remote_selection.contains(&2));
        assert!(!pane.remote_selection.contains(&3));
        assert!(pane.remote_selection.contains(&4));
        assert!(pane.remote_selection.contains(&5));
    }

    #[test]
    fn queue_counts_include_active_and_pending_items() {
        let mut app = app_with(vec![]);
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Upload, 25, 100));
        app.queued_transfers
            .push_back(queued_file_upload_for_test("b.txt"));
        app.queued_transfers
            .push_back(queued_file_upload_for_test("c.txt"));

        let badge = app.global_transfer_badge().unwrap();

        assert_eq!(badge.direction_symbol, "↑");
        assert_eq!(badge.current_index, 1);
        assert_eq!(badge.total_count, 3);
        assert_eq!(badge.percent, 25);
    }

    #[test]
    fn queue_badge_uses_queue_position_instead_of_recursive_file_position() {
        let mut app = app_with(vec![]);
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Download, 50, 100));
        app.queue_completed_count = 1;
        app.queue_total_count = 4;
        app.active_transfer
            .as_mut()
            .unwrap()
            .progress
            .current_file_index = 2;
        app.active_transfer.as_mut().unwrap().progress.total_files = 10;

        let badge = app.global_transfer_badge().unwrap();

        assert_eq!(badge.direction_symbol, "↓");
        assert_eq!(badge.current_index, 2);
        assert_eq!(badge.total_count, 4);
        assert_eq!(badge.percent, 50);
    }

    #[test]
    fn queue_badge_percent_uses_resumed_progress_bytes() {
        let mut app = app_with(vec![]);
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Download, 40, 100));

        let badge = app.global_transfer_badge().unwrap();

        assert_eq!(badge.percent, 40);
    }

    #[tokio::test]
    async fn busy_upload_request_is_enqueued_instead_of_replacing_active_transfer() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), b"a").unwrap();
        std::fs::write(temp.path().join("b.txt"), b"b").unwrap();

        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_path = temp.path().to_path_buf();
        pane.local_entries = vec![file_entry("a.txt", false), file_entry("b.txt", false)];
        pane.local_selection.extend([0, 1]);
        app.mode = AppMode::Sftp;
        app.sftp_pane = Some(pane);
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Upload, 10, 100));
        app.queue_total_count = 1;

        app.start_upload().await.unwrap();

        assert!(app.active_transfer.is_some());
        assert_eq!(app.queued_transfers.len(), 2);
        assert_eq!(app.queue_total_count, 3);
    }

    #[tokio::test]
    async fn busy_download_request_is_enqueued_instead_of_replacing_active_transfer() {
        let temp = tempfile::tempdir().unwrap();

        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Remote;
        pane.local_path = temp.path().to_path_buf();
        pane.remote_entries = vec![file_entry("a.txt", false), file_entry("b.txt", false)];
        pane.remote_selection.extend([0, 1]);
        app.mode = AppMode::Sftp;
        app.sftp_pane = Some(pane);
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Download, 10, 100));
        app.queue_total_count = 1;

        app.start_download().await.unwrap();

        assert!(app.active_transfer.is_some());
        assert_eq!(app.queued_transfers.len(), 2);
        assert_eq!(app.queue_total_count, 3);
    }

    #[test]
    fn completed_transfer_starts_next_queued_job() {
        let mut app = app_with(vec![]);
        app.active_transfer = Some(completed_transfer_for_test(TransferDir::Upload));
        app.queued_transfers
            .push_back(queued_recursive_upload_for_test("next-dir"));
        app.queue_total_count = 2;

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(app.poll_transfer_queue()).unwrap();

        assert!(app.active_recursive_transfer.is_some());
        assert!(app.queued_transfers.is_empty());
        let badge = app.global_transfer_badge().unwrap();
        assert_eq!(badge.current_index, 2);
        assert_eq!(badge.total_count, 2);
    }

    #[test]
    fn leaving_sftp_does_not_stop_background_queue() {
        let mut app = app_with(vec![]);
        app.mode = AppMode::Sftp;
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Download, 10, 100));
        app.queued_transfers
            .push_back(queued_file_download_for_test("next.txt"));

        app.exit_sftp();

        assert_eq!(app.mode, AppMode::Main);
        assert!(app.active_transfer.is_some());
        assert_eq!(app.queued_transfers.len(), 1);
    }

    #[test]
    fn leaving_connection_clears_background_transfer_queue() {
        let mut app = app_with(vec![]);
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Upload, 10, 100));
        app.queued_transfers
            .push_back(queued_file_upload_for_test("next.txt"));
        app.queue_total_count = 2;

        app.clear_connection_state();

        assert!(app.active_transfer.is_none());
        assert!(app.queued_transfers.is_empty());
        assert_eq!(app.queue_total_count, 0);
    }

    #[test]
    fn batch_upload_uses_selected_local_entries() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), b"a").unwrap();
        std::fs::write(temp.path().join("b.txt"), b"b").unwrap();

        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_path = temp.path().to_path_buf();
        pane.local_entries = vec![file_entry("a.txt", false), file_entry("b.txt", false)];
        pane.local_selection.extend([0, 1]);
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Upload, 10, 100));
        app.queue_total_count = 1;

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(app.start_upload()).unwrap();

        assert!(app.active_transfer.is_some());
        assert_eq!(app.queue_total_count, 3);
        assert_eq!(app.queued_transfers.len(), 2);
        let pane = app.sftp_pane.as_ref().unwrap();
        assert!(pane.local_selection.is_empty());
        assert_eq!(pane.local_selection_anchor, None);
    }

    #[test]
    fn batch_download_uses_selected_remote_entries() {
        let temp = tempfile::tempdir().unwrap();

        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Remote;
        pane.local_path = temp.path().to_path_buf();
        pane.remote_entries = vec![file_entry("a.txt", false), file_entry("b.txt", false)];
        pane.remote_selection.extend([0, 1]);
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;
        app.active_transfer = Some(active_transfer_for_test(TransferDir::Download, 10, 100));
        app.queue_total_count = 1;

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(app.start_download()).unwrap();

        assert!(app.active_transfer.is_some());
        assert_eq!(app.queue_total_count, 3);
        assert_eq!(app.queued_transfers.len(), 2);
        let pane = app.sftp_pane.as_ref().unwrap();
        assert!(pane.remote_selection.is_empty());
        assert_eq!(pane.remote_selection_anchor, None);
    }

    #[test]
    fn batch_delete_uses_selected_remote_entries() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Remote;
        pane.remote_entries = vec![file_entry("a.txt", false), file_entry("b.txt", false)];
        pane.remote_selection.extend([0, 1]);
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_sftp_key(KeyEvent::from(KeyCode::Char('D')));

        assert!(app.sftp_delete_confirm.is_some());
    }

    #[test]
    fn esc_clears_multi_select_without_leaving_sftp() {
        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_entries = vec![file_entry("a.txt", false)];
        pane.local_selection.insert(0);
        pane.local_selection_anchor = Some(0);
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.handle_key(KeyEvent::from(KeyCode::Esc));

        assert_eq!(app.mode, AppMode::Sftp);
        let pane = app.sftp_pane.as_ref().unwrap();
        assert!(pane.local_selection.is_empty());
        assert_eq!(pane.local_selection_anchor, None);
    }

    #[test]
    fn confirming_batch_delete_removes_selected_local_entries() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), b"a").unwrap();
        std::fs::write(temp.path().join("b.txt"), b"b").unwrap();

        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_path = temp.path().to_path_buf();
        pane.local_entries = vec![file_entry("a.txt", false), file_entry("b.txt", false)];
        pane.local_selection.extend([0, 1]);
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        app.trigger_sftp_delete_confirm();
        app.handle_sftp_delete_confirm(KeyEvent::from(KeyCode::Char('y')));

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(app.start_sftp_delete()).unwrap();

        assert!(!temp.path().join("a.txt").exists());
        assert!(!temp.path().join("b.txt").exists());
        assert!(app.sftp_delete_confirm.is_none());
        assert!(app.trigger_refresh_local);
        let pane = app.sftp_pane.as_ref().unwrap();
        assert!(pane.local_selection.is_empty());
        assert_eq!(pane.local_selection_anchor, None);
    }

    #[test]
    fn polling_detects_changed_fingerprint() {
        let mut state = RemoteEditWatchState::new("old".into());
        assert!(state.should_upload("new"));
        assert!(!state.should_upload("new"));
    }

    #[test]
    fn repeated_same_fingerprint_does_not_reupload() {
        let mut session = RemoteEditSession::for_test(
            "/remote/app.conf".into(),
            PathBuf::from("/tmp/app.conf"),
            "hash-1".into(),
        );
        assert!(session.should_upload("hash-2"));
        session.mark_uploaded("hash-2".into());
        assert!(!session.should_upload("hash-2"));
    }

    #[test]
    fn failed_upload_retries_after_next_change() {
        let mut session = RemoteEditSession::for_test(
            "/remote/app.conf".into(),
            PathBuf::from("/tmp/app.conf"),
            "hash-1".into(),
        );
        assert!(session.should_upload("hash-2"));
        session.mark_upload_failed("timeout".into());
        assert!(session.should_upload("hash-3"));
    }

    #[test]
    fn conflict_prompt_accepts_overwrite_for_remaining_files() {
        let mut state = ConflictResolutionState::default();
        state.apply_choice(FileConflictChoice::Overwrite, true);
        assert_eq!(
            state.default_file_conflict,
            Some(FileConflictChoice::Overwrite)
        );
    }

    #[test]
    fn conflict_prompt_accepts_skip_without_default() {
        let mut state = ConflictResolutionState::default();
        state.apply_choice(FileConflictChoice::Skip, false);
        assert_eq!(state.default_file_conflict, None);
    }

    #[test]
    fn failure_prompt_retry_keeps_current_file() {
        let mut state = RecursiveFailureState::for_file("a.txt");
        state.apply(FailureChoice::Retry);
        assert!(state.should_retry_current_file);
    }

    #[test]
    fn failure_prompt_skip_advances_to_next_file() {
        let mut state = RecursiveFailureState::for_file("a.txt");
        state.apply(FailureChoice::Skip);
        assert!(state.should_skip_current_file);
    }

    #[test]
    fn failure_prompt_cancel_marks_task_cancelled() {
        let mut state = RecursiveFailureState::for_file("a.txt");
        assert_eq!(state.file_name, "a.txt");
        state.apply(FailureChoice::Cancel);
        assert!(state.should_cancel_task);
    }

    #[test]
    fn uploading_selected_directory_starts_recursive_transfer() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("foo")).unwrap();
        std::fs::write(temp.path().join("foo/a.txt"), b"a").unwrap();

        let mut app = app_with(vec![]);
        let mut pane = SftpPaneState::new("/remote".into());
        pane.side = PaneSide::Local;
        pane.local_path = temp.path().to_path_buf();
        pane.local_entries = vec![crate::sftp::client::FileEntry {
            name: "foo".into(),
            is_dir: true,
            size: 0,
        }];
        pane.local_list_state.select(Some(0));
        app.sftp_pane = Some(pane);
        app.mode = AppMode::Sftp;

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(app.start_upload()).unwrap();

        assert!(app.active_recursive_transfer.is_some());
        assert_eq!(
            app.active_recursive_transfer
                .as_ref()
                .unwrap()
                .plan
                .destination_root,
            "/remote/foo"
        );
    }

    #[test]
    fn recursive_transfer_skip_advances_aggregate_progress() {
        let plan = RecursiveTransferPlan::upload(
            PathBuf::from("/local/foo"),
            "/remote".into(),
            vec![],
            vec![crate::sftp::transfer::PlannedFile {
                relative_path: PathBuf::from("a.txt"),
                size: 10,
            }],
        );

        let mut transfer = ActiveRecursiveTransfer::new(plan);
        transfer.mark_file_pending("a.txt", 10);
        transfer.skip_pending_file();

        assert_eq!(transfer.progress.current_file_index, 1);
        assert_eq!(transfer.next_file_index, 1);
        assert!(transfer.progress.current_file_name.is_none());
    }

    #[test]
    fn recursive_transfer_tracks_current_file_bytes() {
        let plan = RecursiveTransferPlan::upload(
            PathBuf::from("/local/foo"),
            "/remote".into(),
            vec![],
            vec![crate::sftp::transfer::PlannedFile {
                relative_path: PathBuf::from("a.txt"),
                size: 10,
            }],
        );

        let mut transfer = ActiveRecursiveTransfer::new(plan);
        transfer.mark_file_pending("a.txt", 10);
        transfer.update_current_file_bytes(4);

        assert_eq!(
            transfer.progress.current_file_name.as_deref(),
            Some("a.txt")
        );
        assert_eq!(transfer.progress.current_file_bytes, 4);
        assert_eq!(transfer.progress.current_file_total_bytes, 10);
    }

    #[test]
    fn conflict_prompt_toggle_then_overwrite_sets_default_choice() {
        let mut app = app_with(vec![]);
        app.mode = AppMode::Sftp;
        app.recursive_conflict_prompt = Some(RecursiveConflictPrompt::new("a.txt".into()));

        app.handle_key(KeyEvent::from(KeyCode::Char('a')));
        app.handle_key(KeyEvent::from(KeyCode::Char('o')));

        assert!(app.recursive_conflict_prompt.is_none());
        assert_eq!(
            app.conflict_resolution.default_file_conflict,
            Some(FileConflictChoice::Overwrite)
        );
    }

    #[test]
    fn failure_prompt_retry_keeps_current_recursive_file_pending() {
        let plan = RecursiveTransferPlan::upload(
            PathBuf::from("/local/foo"),
            "/remote".into(),
            vec![],
            vec![crate::sftp::transfer::PlannedFile {
                relative_path: PathBuf::from("a.txt"),
                size: 10,
            }],
        );

        let mut app = app_with(vec![]);
        app.mode = AppMode::Sftp;
        app.active_recursive_transfer = Some(ActiveRecursiveTransfer::new(plan));
        app.recursive_failure_prompt = Some(RecursiveFailureState::for_file("a.txt"));

        app.handle_key(KeyEvent::from(KeyCode::Char('r')));

        assert!(app.recursive_failure_prompt.is_none());
        assert_eq!(
            app.active_recursive_transfer
                .as_ref()
                .unwrap()
                .next_file_index,
            0
        );
    }

    #[test]
    fn recursive_download_conflict_opens_prompt_before_transfer() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("foo")).unwrap();
        std::fs::write(temp.path().join("foo/a.txt"), b"old").unwrap();

        let plan = RecursiveTransferPlan::download(
            "/remote/foo".into(),
            temp.path().to_path_buf(),
            vec![],
            vec![crate::sftp::transfer::PlannedFile {
                relative_path: PathBuf::from("a.txt"),
                size: 10,
            }],
        );

        let mut app = app_with(vec![]);
        let mut transfer = ActiveRecursiveTransfer::new(plan);
        transfer.directories_prepared = true;
        app.active_recursive_transfer = Some(transfer);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(app.start_next_recursive_file()).unwrap();

        assert!(app.active_transfer.is_none());
        assert_eq!(
            app.recursive_conflict_prompt
                .as_ref()
                .map(|prompt| prompt.file_name.as_str()),
            Some("a.txt")
        );
    }

    #[test]
    fn failed_recursive_transfer_opens_failure_prompt() {
        let plan = RecursiveTransferPlan::upload(
            PathBuf::from("/local/foo"),
            "/remote".into(),
            vec![],
            vec![crate::sftp::transfer::PlannedFile {
                relative_path: PathBuf::from("a.txt"),
                size: 10,
            }],
        );

        let mut app = app_with(vec![]);
        app.active_recursive_transfer = Some(ActiveRecursiveTransfer::new(plan));
        app.handle_recursive_transfer_result(TransferProgress {
            filename: "a.txt".into(),
            total_bytes: 10,
            transferred_bytes: 3,
            state: TransferState::Failed("boom".into()),
            current_file_index: 1,
            total_files: 1,
        });

        assert!(app.active_recursive_transfer.is_some());
        assert_eq!(
            app.recursive_failure_prompt
                .as_ref()
                .map(|prompt| prompt.file_name.as_str()),
            Some("a.txt")
        );
    }
}
