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

const CHUNK: usize = 32 * 1024;

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
