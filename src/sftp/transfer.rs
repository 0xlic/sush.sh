use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::sftp::client::{FileEntry, SftpClient};

#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub filename: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub state: TransferState,
    pub current_file_index: usize,
    pub total_files: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TransferState {
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}

pub struct TransferHandle {
    pub rx: mpsc::Receiver<TransferProgress>,
    pub cancel: CancellationToken,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecursiveTransferEvent {
    CreateDir { relative_path: PathBuf },
    TransferFile { relative_path: PathBuf, size: u64 },
    FileConflict { relative_path: PathBuf },
    Finished,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RecursiveTransferDriver {
    plan: RecursiveTransferPlan,
    conflicting_files: Vec<PathBuf>,
}

#[allow(dead_code)]
impl RecursiveTransferDriver {
    pub fn new(plan: RecursiveTransferPlan, conflicting_files: Vec<PathBuf>) -> Self {
        Self {
            plan,
            conflicting_files,
        }
    }

    pub fn collect_events(&self) -> Vec<RecursiveTransferEvent> {
        let mut events = Vec::new();
        for directory in &self.plan.directories {
            events.push(RecursiveTransferEvent::CreateDir {
                relative_path: directory.relative_path.clone(),
            });
        }
        for file in &self.plan.files {
            if self.conflicting_files.contains(&file.relative_path) {
                events.push(RecursiveTransferEvent::FileConflict {
                    relative_path: file.relative_path.clone(),
                });
            } else {
                events.push(RecursiveTransferEvent::TransferFile {
                    relative_path: file.relative_path.clone(),
                    size: file.size,
                });
            }
        }
        events.push(RecursiveTransferEvent::Finished);
        events
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDir {
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    pub relative_path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecursiveTransferPlan {
    pub dir: crate::app::TransferDir,
    pub source_root: PathBuf,
    pub destination_root: String,
    pub directories: Vec<PlannedDir>,
    pub files: Vec<PlannedFile>,
}

impl RecursiveTransferPlan {
    pub fn upload(
        source_root: PathBuf,
        destination_parent: String,
        directories: Vec<PlannedDir>,
        files: Vec<PlannedFile>,
    ) -> Self {
        let root_name = source_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let destination_root = join_remote_path(&destination_parent, &root_name);
        Self {
            dir: crate::app::TransferDir::Upload,
            source_root,
            destination_root,
            directories,
            files,
        }
    }

    pub fn download(
        source_root: String,
        destination_parent: PathBuf,
        directories: Vec<PlannedDir>,
        files: Vec<PlannedFile>,
    ) -> Self {
        let root_name = Path::new(&source_root)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let destination_root = destination_parent.join(&root_name);
        Self {
            dir: crate::app::TransferDir::Download,
            source_root: PathBuf::from(source_root),
            destination_root: destination_root.display().to_string(),
            directories,
            files,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecursiveAggregateProgress {
    pub current_file_index: usize,
    pub total_files: usize,
    pub current_file_name: Option<String>,
    pub current_file_total_bytes: u64,
    pub current_file_bytes: u64,
}

impl RecursiveAggregateProgress {
    pub fn new(total_files: usize) -> Self {
        Self {
            current_file_index: 0,
            total_files,
            current_file_name: None,
            current_file_total_bytes: 0,
            current_file_bytes: 0,
        }
    }

    pub fn start_file(&mut self, name: String, total_bytes: u64) {
        self.current_file_name = Some(name);
        self.current_file_total_bytes = total_bytes;
        self.current_file_bytes = 0;
    }

    pub fn update_bytes(&mut self, transferred_bytes: u64) {
        self.current_file_bytes = transferred_bytes;
    }

    pub fn finish_file(&mut self) {
        if self.current_file_index < self.total_files {
            self.current_file_index += 1;
        }
        self.current_file_name = None;
        self.current_file_total_bytes = 0;
        self.current_file_bytes = 0;
    }
}

const CHUNK: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DownloadResumeState {
    offset: u64,
    append: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UploadResumeState {
    offset: u64,
    resume: bool,
}

fn resume_offset_for_download(total_bytes: u64, existing_size: Option<u64>) -> u64 {
    match existing_size {
        Some(size) if size <= total_bytes => size,
        _ => 0,
    }
}

fn resume_offset_for_upload(total_bytes: u64, existing_size: Option<u64>) -> u64 {
    match existing_size {
        Some(size) if size <= total_bytes => size,
        _ => 0,
    }
}

fn build_download_resume_state(local: &Path, total_bytes: u64) -> Result<DownloadResumeState> {
    let existing_size = std::fs::metadata(local).ok().map(|metadata| metadata.len());
    let offset = resume_offset_for_download(total_bytes, existing_size);
    Ok(DownloadResumeState {
        offset,
        append: offset > 0,
    })
}

fn build_upload_resume_state(
    total_bytes: u64,
    existing_remote_size: Option<u64>,
) -> UploadResumeState {
    let offset = resume_offset_for_upload(total_bytes, existing_remote_size);
    UploadResumeState {
        offset,
        resume: offset > 0,
    }
}

pub fn build_local_recursive_plan(
    source_root: &Path,
    destination_parent: &str,
) -> Result<RecursiveTransferPlan> {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    collect_local_entries(source_root, source_root, &mut directories, &mut files)?;
    directories.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(RecursiveTransferPlan::upload(
        source_root.to_path_buf(),
        destination_parent.to_string(),
        directories,
        files,
    ))
}

pub async fn build_remote_recursive_plan(
    client: &SftpClient,
    source_root: &str,
    destination_parent: &Path,
) -> Result<RecursiveTransferPlan> {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    collect_remote_entries(
        client,
        source_root,
        source_root,
        &mut directories,
        &mut files,
    )
    .await?;
    directories.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(RecursiveTransferPlan::download(
        source_root.to_string(),
        destination_parent.to_path_buf(),
        directories,
        files,
    ))
}

#[allow(dead_code)]
pub fn build_local_batch_plan(
    source_root: &Path,
    destination_root: &str,
    selected_entries: &[FileEntry],
) -> Result<RecursiveTransferPlan> {
    let mut directories = Vec::new();
    let mut files = Vec::new();

    for entry in selected_entries {
        let relative_path = PathBuf::from(&entry.name);
        if entry.is_dir {
            directories.push(PlannedDir {
                relative_path: relative_path.clone(),
            });
            collect_local_entries(
                source_root,
                &source_root.join(&entry.name),
                &mut directories,
                &mut files,
            )?;
        } else {
            files.push(PlannedFile {
                relative_path,
                size: entry.size,
            });
        }
    }

    directories.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    Ok(RecursiveTransferPlan {
        dir: crate::app::TransferDir::Upload,
        source_root: source_root.to_path_buf(),
        destination_root: destination_root.to_string(),
        directories,
        files,
    })
}

#[allow(dead_code)]
pub async fn build_remote_batch_plan(
    client: Option<&SftpClient>,
    source_root: &str,
    destination_root: &Path,
    selected_entries: &[FileEntry],
) -> Result<RecursiveTransferPlan> {
    let mut directories = Vec::new();
    let mut files = Vec::new();

    for entry in selected_entries {
        let relative_path = PathBuf::from(&entry.name);
        if entry.is_dir {
            directories.push(PlannedDir {
                relative_path: relative_path.clone(),
            });
            let client = client.context("missing SFTP client for batch remote directory plan")?;
            let remote_root = join_remote_path(source_root, &entry.name);
            collect_remote_entries(
                client,
                source_root,
                &remote_root,
                &mut directories,
                &mut files,
            )
            .await?;
        } else {
            files.push(PlannedFile {
                relative_path,
                size: entry.size,
            });
        }
    }

    directories.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    Ok(RecursiveTransferPlan {
        dir: crate::app::TransferDir::Download,
        source_root: PathBuf::from(source_root),
        destination_root: destination_root.display().to_string(),
        directories,
        files,
    })
}

pub fn download(sftp: SftpSession, remote: String, local: PathBuf, total: u64) -> TransferHandle {
    let (tx, rx) = mpsc::channel(32);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        let r = do_download(&sftp, &remote, &local, total, tx.clone(), c2).await;
        if let Err(e) = r {
            let _ = tx
                .send(TransferProgress {
                    filename: remote.clone(),
                    total_bytes: total,
                    transferred_bytes: 0,
                    state: TransferState::Failed(e.to_string()),
                    current_file_index: 1,
                    total_files: 1,
                })
                .await;
        }
    });
    TransferHandle { rx, cancel }
}

async fn do_download(
    sftp: &SftpSession,
    remote: &str,
    local: &Path,
    total: u64,
    tx: mpsc::Sender<TransferProgress>,
    cancel: CancellationToken,
) -> Result<()> {
    let filename = Path::new(remote)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let resume_state = build_download_resume_state(local, total)?;
    let mut rf = sftp
        .open(remote)
        .await
        .context("failed to open remote file")?;
    if resume_state.offset > 0 {
        rf.seek(SeekFrom::Start(resume_state.offset)).await?;
    }
    let mut lf = if resume_state.append {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(local)
            .await?
    } else {
        tokio::fs::File::create(local).await?
    };
    let mut buf = vec![0u8; CHUNK];
    let mut acc = resume_state.offset;
    loop {
        if cancel.is_cancelled() {
            let _ = tx
                .send(TransferProgress {
                    filename: filename.clone(),
                    total_bytes: total,
                    transferred_bytes: acc,
                    state: TransferState::Cancelled,
                    current_file_index: 1,
                    total_files: 1,
                })
                .await;
            return Ok(());
        }
        let n = rf.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        lf.write_all(&buf[..n]).await?;
        acc += n as u64;
        let _ = tx
            .send(TransferProgress {
                filename: filename.clone(),
                total_bytes: total,
                transferred_bytes: acc,
                state: TransferState::InProgress,
                current_file_index: 1,
                total_files: 1,
            })
            .await;
    }
    let _ = tx
        .send(TransferProgress {
            filename,
            total_bytes: total,
            transferred_bytes: acc,
            state: TransferState::Completed,
            current_file_index: 1,
            total_files: 1,
        })
        .await;
    Ok(())
}

pub fn upload(sftp: SftpSession, local: PathBuf, remote: String) -> Result<TransferHandle> {
    let total = std::fs::metadata(&local)?.len();
    let (tx, rx) = mpsc::channel(32);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        let r = do_upload(&sftp, &local, &remote, total, tx.clone(), c2).await;
        if let Err(e) = r {
            let _ = tx
                .send(TransferProgress {
                    filename: remote.clone(),
                    total_bytes: total,
                    transferred_bytes: 0,
                    state: TransferState::Failed(e.to_string()),
                    current_file_index: 1,
                    total_files: 1,
                })
                .await;
        }
    });
    Ok(TransferHandle { rx, cancel })
}

async fn do_upload(
    sftp: &SftpSession,
    local: &Path,
    remote: &str,
    total: u64,
    tx: mpsc::Sender<TransferProgress>,
    cancel: CancellationToken,
) -> Result<()> {
    let filename = local
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut lf = tokio::fs::File::open(local).await?;
    let remote_size = sftp
        .metadata(remote)
        .await
        .ok()
        .and_then(|metadata| metadata.size);
    let resume_state = build_upload_resume_state(total, remote_size);
    if resume_state.offset > 0 {
        lf.seek(SeekFrom::Start(resume_state.offset)).await?;
    }
    let mut rf = if resume_state.resume {
        sftp.open_with_flags(
            remote,
            OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::APPEND,
        )
        .await
        .context("failed to open remote file for resume")?
    } else {
        sftp.create(remote)
            .await
            .context("failed to create remote file")?
    };
    let mut buf = vec![0u8; CHUNK];
    let mut acc = resume_state.offset;
    loop {
        if cancel.is_cancelled() {
            let _ = tx
                .send(TransferProgress {
                    filename: filename.clone(),
                    total_bytes: total,
                    transferred_bytes: acc,
                    state: TransferState::Cancelled,
                    current_file_index: 1,
                    total_files: 1,
                })
                .await;
            return Ok(());
        }
        let n = lf.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        rf.write_all(&buf[..n]).await?;
        acc += n as u64;
        let _ = tx
            .send(TransferProgress {
                filename: filename.clone(),
                total_bytes: total,
                transferred_bytes: acc,
                state: TransferState::InProgress,
                current_file_index: 1,
                total_files: 1,
            })
            .await;
    }
    let _ = tx
        .send(TransferProgress {
            filename,
            total_bytes: total,
            transferred_bytes: acc,
            state: TransferState::Completed,
            current_file_index: 1,
            total_files: 1,
        })
        .await;
    Ok(())
}

fn join_remote_path(parent: &str, child: &str) -> String {
    match parent {
        "/" => format!("/{child}"),
        _ if parent.ends_with('/') => format!("{parent}{child}"),
        _ => format!("{parent}/{child}"),
    }
}

fn collect_local_entries(
    root: &Path,
    current: &Path,
    directories: &mut Vec<PlannedDir>,
    files: &mut Vec<PlannedFile>,
) -> Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        let relative_path = path
            .strip_prefix(root)
            .context("failed to derive relative path for recursive transfer")?
            .to_path_buf();
        if metadata.is_dir() {
            directories.push(PlannedDir {
                relative_path: relative_path.clone(),
            });
            collect_local_entries(root, &path, directories, files)?;
        } else if metadata.is_file() {
            files.push(PlannedFile {
                relative_path,
                size: metadata.len(),
            });
        }
    }
    Ok(())
}

async fn collect_remote_entries(
    client: &SftpClient,
    root: &str,
    current: &str,
    directories: &mut Vec<PlannedDir>,
    files: &mut Vec<PlannedFile>,
) -> Result<()> {
    let mut pending_dirs = Vec::new();
    for entry in client.list_dir(current).await? {
        if matches!(entry.name.as_str(), "." | "..") {
            continue;
        }

        let child_path = join_remote_path(current, &entry.name);
        let relative_path = Path::new(&child_path)
            .strip_prefix(root)
            .context("failed to derive remote relative path for recursive transfer")?
            .to_path_buf();

        if entry.is_dir {
            directories.push(PlannedDir {
                relative_path: relative_path.clone(),
            });
            pending_dirs.push(child_path);
        } else {
            files.push(PlannedFile {
                relative_path,
                size: entry.size,
            });
        }
    }

    for directory in pending_dirs {
        Box::pin(collect_remote_entries(
            client,
            root,
            &directory,
            directories,
            files,
        ))
        .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn upload_plan_keeps_selected_directory_name() {
        let plan = RecursiveTransferPlan::upload(
            PathBuf::from("/local/foo"),
            "/remote".into(),
            vec![
                PlannedDir {
                    relative_path: PathBuf::from("bar"),
                },
                PlannedDir {
                    relative_path: PathBuf::from("baz"),
                },
            ],
            vec![PlannedFile {
                relative_path: PathBuf::from("a.txt"),
                size: 10,
            }],
        );
        assert_eq!(plan.destination_root, "/remote/foo");
    }

    #[test]
    fn aggregate_progress_counts_only_files() {
        let progress = RecursiveAggregateProgress::new(3);
        assert_eq!(progress.total_files, 3);
        assert_eq!(progress.current_file_index, 0);
    }

    #[test]
    fn download_plan_keeps_selected_directory_name() {
        let plan = RecursiveTransferPlan::download(
            "/remote/foo".into(),
            PathBuf::from("/tmp/local"),
            vec![PlannedDir {
                relative_path: PathBuf::from("bar"),
            }],
            vec![PlannedFile {
                relative_path: PathBuf::from("a.txt"),
                size: 10,
            }],
        );
        assert_eq!(plan.destination_root, "/tmp/local/foo");
        assert_eq!(plan.dir, crate::app::TransferDir::Download);
    }

    #[test]
    fn local_scan_collects_nested_files() {
        let temp = tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("foo/bar")).unwrap();
        std::fs::write(temp.path().join("foo/a.txt"), b"a").unwrap();
        std::fs::write(temp.path().join("foo/bar/b.txt"), b"bb").unwrap();

        let plan = build_local_recursive_plan(&temp.path().join("foo"), "/remote").unwrap();

        assert_eq!(plan.destination_root, "/remote/foo");
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.directories.len(), 1);
        assert_eq!(plan.directories[0].relative_path, PathBuf::from("bar"));
        assert_eq!(plan.files[0].relative_path, PathBuf::from("a.txt"));
        assert_eq!(plan.files[1].relative_path, PathBuf::from("bar/b.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn local_scan_does_not_follow_directory_symlinks() {
        let temp = tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("foo/real")).unwrap();
        std::fs::write(temp.path().join("foo/real/a.txt"), b"a").unwrap();
        std::os::unix::fs::symlink(
            temp.path().join("foo/real"),
            temp.path().join("foo/link-to-real"),
        )
        .unwrap();

        let plan = build_local_recursive_plan(&temp.path().join("foo"), "/remote").unwrap();

        assert_eq!(
            plan.directories,
            vec![PlannedDir {
                relative_path: PathBuf::from("real")
            }]
        );
        assert_eq!(
            plan.files,
            vec![PlannedFile {
                relative_path: PathBuf::from("real/a.txt"),
                size: 1,
            }]
        );
    }

    #[test]
    fn aggregate_progress_moves_to_next_file() {
        let mut progress = RecursiveAggregateProgress::new(2);
        progress.start_file("a.txt".into(), 10);
        progress.finish_file();
        assert_eq!(progress.current_file_index, 1);
    }

    #[test]
    fn aggregate_progress_keeps_current_file_bytes() {
        let mut progress = RecursiveAggregateProgress::new(1);
        progress.start_file("a.txt".into(), 10);
        progress.update_bytes(4);
        assert_eq!(progress.current_file_bytes, 4);
        assert_eq!(progress.current_file_name.as_deref(), Some("a.txt"));
        assert_eq!(progress.current_file_total_bytes, 10);
    }

    #[test]
    fn recursive_driver_creates_directories_before_files() {
        let plan = RecursiveTransferPlan::upload(
            PathBuf::from("/local/foo"),
            "/remote".into(),
            vec![PlannedDir {
                relative_path: PathBuf::from("bar"),
            }],
            vec![PlannedFile {
                relative_path: PathBuf::from("bar/a.txt"),
                size: 10,
            }],
        );
        let steps = RecursiveTransferDriver::new(plan, vec![]).collect_events();
        assert!(matches!(steps[0], RecursiveTransferEvent::CreateDir { .. }));
        assert!(matches!(
            steps[1],
            RecursiveTransferEvent::TransferFile { .. }
        ));
    }

    #[test]
    fn recursive_driver_pauses_on_file_conflict() {
        let plan = RecursiveTransferPlan::upload(
            PathBuf::from("/local/foo"),
            "/remote".into(),
            vec![],
            vec![PlannedFile {
                relative_path: PathBuf::from("bar/a.txt"),
                size: 10,
            }],
        );
        let steps =
            RecursiveTransferDriver::new(plan, vec![PathBuf::from("bar/a.txt")]).collect_events();
        assert!(matches!(
            steps[0],
            RecursiveTransferEvent::FileConflict { .. }
        ));
    }

    #[test]
    fn download_resume_uses_existing_local_partial_size() {
        assert_eq!(resume_offset_for_download(100, Some(40)), 40);
    }

    #[test]
    fn upload_resume_uses_existing_remote_partial_size() {
        assert_eq!(resume_offset_for_upload(100, Some(40)), 40);
    }

    #[test]
    fn resume_offset_resets_when_target_is_larger_than_source() {
        assert_eq!(resume_offset_for_download(100, Some(140)), 0);
        assert_eq!(resume_offset_for_upload(100, Some(140)), 0);
    }

    #[test]
    fn local_resume_mode_appends_when_partial_file_exists() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("partial.bin");
        std::fs::write(&target, b"abcd").unwrap();

        let state = build_download_resume_state(&target, 10).unwrap();

        assert_eq!(state.offset, 4);
        assert!(state.append);
    }

    #[test]
    fn upload_resume_starts_from_existing_remote_size() {
        let state = build_upload_resume_state(10, Some(4));

        assert_eq!(state.offset, 4);
        assert!(state.resume);
    }
}
