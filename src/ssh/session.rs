use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use russh::client::{self, Handle, Msg};
use russh::keys::PrivateKeyWithHashAlg;
use russh::{Channel, ChannelMsg, Disconnect};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

pub struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        // v0.1: TOFU，一律接受；v0.3 引入 known_hosts 校验
        Ok(true)
    }
}

pub struct ActiveSession {
    pub handle: Handle<ClientHandler>,
    pub channel: Option<Channel<Msg>>,
}

impl ActiveSession {
    pub async fn connect(hostname: &str, port: u16) -> Result<Self> {
        let config = Arc::new(client::Config::default());
        let handle = client::connect(config, (hostname, port), ClientHandler)
            .await
            .with_context(|| format!("连接 {hostname}:{port} 失败"))?;
        Ok(Self {
            handle,
            channel: None,
        })
    }

    /// 打开 PTY channel 并请求 shell。
    pub async fn request_pty(&mut self, cols: u16, rows: u16) -> Result<()> {
        let term = {
            let t = std::env::var("TERM").unwrap_or_default();
            if matches!(
                t.as_str(),
                "xterm"
                    | "xterm-256color"
                    | "screen"
                    | "screen-256color"
                    | "tmux"
                    | "tmux-256color"
                    | "linux"
                    | "vt100"
            ) {
                t
            } else {
                "xterm-256color".into()
            }
        };
        let ch = self.handle.channel_open_session().await?;
        ch.request_pty(false, &term, cols as u32, rows as u32, 0, 0, &[])
            .await?;
        ch.request_shell(false).await?;
        self.channel = Some(ch);
        Ok(())
    }

    /// I/O 接管循环：stdin → 远程 PTY，远程输出 → stdout。
    /// 检测到 `switch_seq`（Shift+Tab = \x1b[Z）时返回 Ok(true)。
    /// 远程关闭/exit 时返回 Ok(false)。
    pub async fn takeover(&mut self, switch_seq: &[u8]) -> Result<bool> {
        let ch = self.channel.as_mut().context("PTY 未建立")?;

        // 用 spawn_blocking 阻塞读 stdin，通过 mpsc 向事件循环投递
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 4096];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if stdin_tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let mut size_ticker = tokio::time::interval(Duration::from_millis(500));
        let mut last_size = crossterm::terminal::size().unwrap_or((80, 24));

        loop {
            tokio::select! {
                Some(data) = stdin_rx.recv() => {
                    if data.windows(switch_seq.len()).any(|w| w == switch_seq) {
                        return Ok(true);
                    }
                    ch.data(data.as_slice()).await?;
                }
                msg = ch.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { ref data }) => {
                            let mut stdout = tokio::io::stdout();
                            stdout.write_all(data).await?;
                            stdout.flush().await?;
                        }
                        Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                            let mut stderr = tokio::io::stderr();
                            stderr.write_all(data).await?;
                            stderr.flush().await?;
                        }
                        Some(ChannelMsg::ExitStatus { .. })
                        | Some(ChannelMsg::Eof)
                        | None => break,
                        _ => {}
                    }
                }
                _ = size_ticker.tick() => {
                    let now = crossterm::terminal::size().unwrap_or(last_size);
                    if now != last_size {
                        ch.window_change(now.0 as u32, now.1 as u32, 0, 0).await?;
                        last_size = now;
                    }
                }
            }
        }
        Ok(false)
    }

    pub async fn disconnect(self) -> Result<()> {
        self.handle
            .disconnect(Disconnect::ByApplication, "", "English")
            .await?;
        Ok(())
    }
}

pub async fn try_key_auth(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    path: &std::path::Path,
    passphrase: Option<&str>,
) -> Result<bool> {
    let key = russh::keys::load_secret_key(path, passphrase)
        .with_context(|| format!("加载私钥失败: {}", path.display()))?;
    let hash_alg = handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    let wrapped = PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
    let res = handle.authenticate_publickey(user, wrapped).await?;
    Ok(res.success())
}
