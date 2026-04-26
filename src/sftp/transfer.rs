use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub filename: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub state: TransferState,
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
    let mut rf = sftp
        .open(remote)
        .await
        .context("failed to open remote file")?;
    let mut lf = tokio::fs::File::create(local).await?;
    let mut buf = vec![0u8; CHUNK];
    let mut acc = 0u64;
    loop {
        if cancel.is_cancelled() {
            let _ = tx
                .send(TransferProgress {
                    filename: filename.clone(),
                    total_bytes: total,
                    transferred_bytes: acc,
                    state: TransferState::Cancelled,
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
            })
            .await;
    }
    let _ = tx
        .send(TransferProgress {
            filename,
            total_bytes: total,
            transferred_bytes: acc,
            state: TransferState::Completed,
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
    let mut rf = sftp
        .create(remote)
        .await
        .context("failed to create remote file")?;
    let mut buf = vec![0u8; CHUNK];
    let mut acc = 0u64;
    loop {
        if cancel.is_cancelled() {
            let _ = tx
                .send(TransferProgress {
                    filename: filename.clone(),
                    total_bytes: total,
                    transferred_bytes: acc,
                    state: TransferState::Cancelled,
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
            })
            .await;
    }
    let _ = tx
        .send(TransferProgress {
            filename,
            total_bytes: total,
            transferred_bytes: acc,
            state: TransferState::Completed,
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

        assert_eq!(plan.directories, vec![PlannedDir { relative_path: PathBuf::from("real") }]);
        assert_eq!(plan.files, vec![PlannedFile {
            relative_path: PathBuf::from("real/a.txt"),
            size: 1,
        }]);
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
}
