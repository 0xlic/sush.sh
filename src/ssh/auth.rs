#[cfg(unix)]
use anyhow::Result;
#[cfg(unix)]
use russh::keys::agent::AgentIdentity;
#[cfg(unix)]
use russh::keys::agent::client::AgentClient;
use std::path::{Path, PathBuf};

use crate::config::host::Host;
use crate::config::secrets::{SecretKey, SecretKind, SecretStore, SystemSecretBackend};

use super::session::ClientHandler;
use super::session::try_key_auth;

#[cfg(unix)]
pub async fn try_agent_auth(
    handle: &mut russh::client::Handle<ClientHandler>,
    user: &str,
) -> Result<bool> {
    let mut agent = match AgentClient::connect_env().await {
        Ok(a) => a,
        Err(_) => return Ok(false),
    };
    let identities = agent.request_identities().await.unwrap_or_default();
    for identity in identities {
        let pub_key = match &identity {
            AgentIdentity::PublicKey { key, .. } => key.clone(),
            AgentIdentity::Certificate { certificate, .. } => {
                certificate.public_key().clone().into()
            }
        };
        let hash_alg = handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten();
        if let Ok(res) = handle
            .authenticate_publickey_with(user, pub_key, hash_alg, &mut agent)
            .await
            && res.success()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn authenticate(
    handle: &mut russh::client::Handle<ClientHandler>,
    host: &Host,
) -> anyhow::Result<()> {
    #[cfg(unix)]
    if try_agent_auth(handle, &host.user).await.unwrap_or(false) {
        return Ok(());
    }

    for path in &host.identity_files {
        let expanded = expand_tilde(path);
        if try_key_auth(handle, &host.user, &expanded, None)
            .await
            .unwrap_or(false)
        {
            return Ok(());
        }
    }

    let secret_store = SecretStore::new(Box::new(SystemSecretBackend::new()));
    let password_key = SecretKey::new(&host.id, SecretKind::LoginPassword, None);
    if let Ok(Some(pass)) = secret_store.get(&password_key)
        && handle
            .authenticate_password(&host.user, &pass)
            .await?
            .success()
    {
        return Ok(());
    }

    anyhow::bail!("all authentication methods failed for {}", host.alias)
}

fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}
