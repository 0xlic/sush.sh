use anyhow::{Context, Result};
use russh::client::{self, Handle, Msg};
use russh::keys::PrivateKeyWithHashAlg;
use russh::{Channel, ChannelMsg, Disconnect};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

pub struct ClientHandler {
    forwarded_tcpip_tx: Option<UnboundedSender<Channel<Msg>>>,
}

impl ClientHandler {
    pub fn new() -> Self {
        Self {
            forwarded_tcpip_tx: None,
        }
    }

    pub fn with_forwarded_tcpip(tx: UnboundedSender<Channel<Msg>>) -> Self {
        Self {
            forwarded_tcpip_tx: Some(tx),
        }
    }
}

impl Default for ClientHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        // v0.1: TOFU, accept all keys; v0.3 may add known_hosts verification.
        Ok(true)
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if let Some(tx) = &self.forwarded_tcpip_tx {
            let _ = tx.send(channel);
        }
        Ok(())
    }
}

pub struct ActiveSession {
    pub handle: Handle<ClientHandler>,
    pub channel: Option<Channel<Msg>>,
}

impl ActiveSession {
    pub async fn connect(hostname: &str, port: u16) -> Result<Self> {
        Self::connect_with_handler(hostname, port, ClientHandler::default()).await
    }

    pub async fn connect_with_handler(
        hostname: &str,
        port: u16,
        handler: ClientHandler,
    ) -> Result<Self> {
        let config = Arc::new(client::Config::default());
        let handle = client::connect(config, (hostname, port), handler)
            .await
            .with_context(|| format!("failed to connect to {hostname}:{port}"))?;
        Ok(Self {
            handle,
            channel: None,
        })
    }

    /// Open a PTY channel and request a shell.
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

    pub fn has_pty(&self) -> bool {
        self.channel.is_some()
    }

    pub async fn write_input(&mut self, data: &[u8]) -> Result<()> {
        let ch = self.channel.as_mut().context("PTY is not established")?;
        ch.data(data).await?;
        Ok(())
    }

    pub async fn resize_pty(&mut self, cols: u16, rows: u16) -> Result<()> {
        let ch = self.channel.as_mut().context("PTY is not established")?;
        ch.window_change(cols as u32, rows as u32, 0, 0).await?;
        Ok(())
    }

    pub async fn wait_channel_msg(&mut self) -> Result<Option<ChannelMsg>> {
        let ch = self.channel.as_mut().context("PTY is not established")?;
        Ok(ch.wait().await)
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
        .with_context(|| format!("failed to load private key: {}", path.display()))?;
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
