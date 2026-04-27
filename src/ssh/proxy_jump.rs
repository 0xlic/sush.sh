use std::sync::Arc;

use anyhow::{Context, Result};
use russh::client::{self, Handle};

use crate::config::host::Host;
use crate::ssh::auth;
use crate::ssh::session::ClientHandler;

#[allow(dead_code)]
pub async fn connect_via_proxy_jump(
    bastion: &Host,
    target: &Host,
) -> Result<Handle<ClientHandler>> {
    connect_via_proxy_jump_with_handler(bastion, target, ClientHandler::default()).await
}

pub async fn connect_via_proxy_jump_with_handler(
    bastion: &Host,
    target: &Host,
    handler: ClientHandler,
) -> Result<Handle<ClientHandler>> {
    let bastion_config = Arc::new(client::Config::default());
    let mut bastion_handle = client::connect(
        bastion_config,
        (bastion.hostname.as_str(), bastion.port),
        ClientHandler::default(),
    )
    .await
    .with_context(|| {
        format!(
            "proxy jump: failed to connect to bastion {}",
            bastion.hostname
        )
    })?;

    auth::authenticate(&mut bastion_handle, bastion)
        .await
        .with_context(|| {
            format!(
                "proxy jump: authentication failed for bastion {}",
                bastion.alias
            )
        })?;

    let channel = bastion_handle
        .channel_open_direct_tcpip(&target.hostname, target.port as u32, "127.0.0.1", 0)
        .await
        .with_context(|| {
            format!(
                "proxy jump: failed to open channel to {}:{}",
                target.hostname, target.port
            )
        })?;

    let channel_stream = channel.into_stream();
    let target_config = Arc::new(client::Config::default());
    let mut target_handle = client::connect_stream(target_config, channel_stream, handler)
        .await
        .with_context(|| {
            format!(
                "proxy jump: SSH handshake failed with target {}",
                target.hostname
            )
        })?;

    auth::authenticate(&mut target_handle, target)
        .await
        .with_context(|| {
            format!(
                "proxy jump: authentication failed for target {}",
                target.alias
            )
        })?;

    Ok(target_handle)
}
