use anyhow::Result;

use crate::config::store;
use crate::tunnel::ipc::{ForwardStatus, IpcRequest, IpcResponse, encode_request};

#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixStream;

fn daemon_sock_path() -> Result<std::path::PathBuf> {
    Ok(store::config_dir().join("daemon.sock"))
}

#[cfg(unix)]
async fn send_request(req: &IpcRequest) -> Result<IpcResponse> {
    let sock = daemon_sock_path()?;
    let mut stream = UnixStream::connect(&sock)
        .await
        .map_err(|e| anyhow::anyhow!("cannot connect to daemon: {e}"))?;

    stream.write_all(&encode_request(req)?).await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let resp: IpcResponse = serde_json::from_str(line.trim())?;
    Ok(resp)
}

#[cfg(unix)]
async fn ensure_daemon_running() -> Result<()> {
    let sock = daemon_sock_path()?;
    if send_request(&IpcRequest::Status).await.is_ok() {
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg("--daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if sock.exists() && send_request(&IpcRequest::Status).await.is_ok() {
            return Ok(());
        }
    }

    anyhow::bail!("daemon socket did not become ready")
}

#[cfg(unix)]
pub async fn daemon_status() -> Vec<ForwardStatus> {
    if ensure_daemon_running().await.is_err() {
        return vec![];
    }
    match send_request(&IpcRequest::Status).await {
        Ok(IpcResponse::Status(statuses)) => statuses,
        _ => vec![],
    }
}

#[cfg(unix)]
pub async fn daemon_start(forward_id: &str) -> Result<()> {
    ensure_daemon_running().await?;
    match send_request(&IpcRequest::Start {
        forward_id: forward_id.into(),
    })
    .await?
    {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error { message } => anyhow::bail!(message),
        IpcResponse::Status(_) => anyhow::bail!("unexpected daemon status response"),
    }
}

#[cfg(unix)]
pub async fn daemon_stop(forward_id: &str) -> Result<()> {
    match send_request(&IpcRequest::Stop {
        forward_id: forward_id.into(),
    })
    .await?
    {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error { message } => anyhow::bail!(message),
        IpcResponse::Status(_) => anyhow::bail!("unexpected daemon status response"),
    }
}

#[cfg(not(unix))]
pub async fn daemon_status() -> Vec<ForwardStatus> {
    vec![]
}

#[cfg(not(unix))]
pub async fn daemon_start(_id: &str) -> Result<()> {
    Ok(())
}

#[cfg(not(unix))]
pub async fn daemon_stop(_id: &str) -> Result<()> {
    Ok(())
}
