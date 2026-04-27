use anyhow::Result;
use russh::ChannelMsg;
use russh::client::{Handle, Msg};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

use crate::ssh::session::ClientHandler;

pub async fn run_local_forward(
    handle: Handle<ClientHandler>,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
    cancel: CancellationToken,
) -> Result<()> {
    let handle = Arc::new(handle);
    let listener = TcpListener::bind(format!("127.0.0.1:{local_port}"))
        .await
        .map_err(|e| anyhow::anyhow!("port {local_port} already in use: {e}"))?;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accept = listener.accept() => {
                let (tcp_stream, _) = accept?;
                let handle = handle.clone();
                let remote_host = remote_host.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    if let Err(e) = forward_local_connection(
                        tcp_stream,
                        handle,
                        &remote_host,
                        remote_port,
                        cancel,
                    ).await {
                        eprintln!("local forward connection error: {e}");
                    }
                });
            }
        }
    }

    Ok(())
}

async fn forward_local_connection(
    tcp: TcpStream,
    handle: Arc<Handle<ClientHandler>>,
    remote_host: &str,
    remote_port: u16,
    cancel: CancellationToken,
) -> Result<()> {
    let channel = handle
        .channel_open_direct_tcpip(remote_host, remote_port as u32, "127.0.0.1", 0)
        .await?;

    pipe_channel(tcp, channel, cancel).await
}

pub async fn run_dynamic_forward(
    handle: Handle<ClientHandler>,
    local_port: u16,
    cancel: CancellationToken,
) -> Result<()> {
    let handle = Arc::new(handle);
    let listener = TcpListener::bind(format!("127.0.0.1:{local_port}"))
        .await
        .map_err(|e| anyhow::anyhow!("port {local_port} already in use: {e}"))?;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _) = accept?;
                let handle = handle.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_socks5(stream, handle, cancel).await {
                        eprintln!("socks5 error: {e}");
                    }
                });
            }
        }
    }

    Ok(())
}

async fn handle_socks5(
    mut stream: TcpStream,
    handle: Arc<Handle<ClientHandler>>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await?;
    anyhow::ensure!(header[0] == 5, "unsupported SOCKS version {}", header[0]);

    let method_count = header[1] as usize;
    let mut methods = vec![0u8; method_count];
    stream.read_exact(&mut methods).await?;
    stream.write_all(&[5, 0]).await?;

    let mut req_hdr = [0u8; 4];
    stream.read_exact(&mut req_hdr).await?;
    anyhow::ensure!(req_hdr[1] == 1, "only CONNECT (0x01) supported");

    let dest_host = match req_hdr[3] {
        1 => {
            let mut ip = [0u8; 4];
            stream.read_exact(&mut ip).await?;
            format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
        }
        3 => {
            let mut len_byte = [0u8; 1];
            stream.read_exact(&mut len_byte).await?;
            let mut name = vec![0u8; len_byte[0] as usize];
            stream.read_exact(&mut name).await?;
            String::from_utf8(name)?
        }
        4 => {
            let mut ip6 = [0u8; 16];
            stream.read_exact(&mut ip6).await?;
            std::net::Ipv6Addr::from(ip6).to_string()
        }
        t => anyhow::bail!("unsupported address type 0x{t:02x}"),
    };

    let mut port_bytes = [0u8; 2];
    stream.read_exact(&mut port_bytes).await?;
    let dest_port = u16::from_be_bytes(port_bytes);

    let channel = handle
        .channel_open_direct_tcpip(&dest_host, dest_port as u32, "127.0.0.1", 0)
        .await
        .map_err(|e| anyhow::anyhow!("cannot connect to {dest_host}:{dest_port}: {e}"))?;

    stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;

    pipe_channel(stream, channel, cancel).await
}

#[allow(dead_code)]
pub async fn run_remote_forward(
    handle: Handle<ClientHandler>,
    remote_host: String,
    remote_port: u16,
    local_port: u16,
    cancel: CancellationToken,
) -> Result<()> {
    let (_, forwarded_rx) = tokio::sync::mpsc::unbounded_channel();
    run_remote_forward_with_receiver(
        handle,
        forwarded_rx,
        remote_host,
        remote_port,
        local_port,
        cancel,
    )
    .await
}

pub async fn run_remote_forward_with_receiver(
    handle: Handle<ClientHandler>,
    mut forwarded_rx: UnboundedReceiver<russh::Channel<Msg>>,
    remote_host: String,
    remote_port: u16,
    local_port: u16,
    cancel: CancellationToken,
) -> Result<()> {
    handle
        .tcpip_forward(&remote_host, remote_port as u32)
        .await
        .map_err(|e| anyhow::anyhow!("tcpip-forward request failed: {e}"))?;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            maybe_channel = forwarded_rx.recv() => {
                let Some(channel) = maybe_channel else {
                    break;
                };
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    match TcpStream::connect(("127.0.0.1", local_port)).await {
                        Ok(tcp) => {
                            if let Err(e) = pipe_channel(tcp, channel, cancel).await {
                                eprintln!("remote forward connection error: {e}");
                            }
                        }
                        Err(e) => {
                            eprintln!("remote forward local connect error: {e}");
                        }
                    }
                });
            }
        }
    }

    handle
        .cancel_tcpip_forward(&remote_host, remote_port as u32)
        .await
        .ok();
    Ok(())
}

async fn pipe_channel(
    mut tcp: TcpStream,
    mut ch: russh::Channel<Msg>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            n = tcp.read(&mut buf) => {
                let n = n?;
                if n == 0 {
                    ch.eof().await.ok();
                    break;
                }
                ch.data(&buf[..n]).await?;
            }
            msg = ch.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => tcp.write_all(&data).await?,
                    Some(ChannelMsg::Eof) | None => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
