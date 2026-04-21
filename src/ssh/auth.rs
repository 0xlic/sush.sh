use std::path::PathBuf;

use anyhow::Result;
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;

use super::session::{ActiveSession, ClientHandler, try_key_auth};
use crate::config::host::Host;

pub type PasswordPrompt = Box<dyn FnMut(&str) -> Option<String> + Send>;

/// 认证链：agent → IdentityFile（无密码/有密码）→ 密码认证。
pub async fn connect_with_host(host: &Host, mut prompt: PasswordPrompt) -> Result<ActiveSession> {
    let mut session = ActiveSession::connect(&host.hostname, host.port).await?;

    // 1. ssh-agent
    if try_agent_auth(&mut session.handle, &host.user)
        .await
        .unwrap_or(false)
    {
        return Ok(session);
    }

    // 2. IdentityFile 序列
    for key_path in &host.identity_files {
        let expanded = expand_tilde(key_path);
        if try_key_auth(&mut session.handle, &host.user, &expanded, None)
            .await
            .unwrap_or(false)
        {
            return Ok(session);
        }
        if let Some(pass) = prompt(&format!("密钥密码 ({}): ", expanded.display())) {
            if try_key_auth(&mut session.handle, &host.user, &expanded, Some(&pass))
                .await
                .unwrap_or(false)
            {
                return Ok(session);
            }
        }
    }

    // 3. 密码认证
    if let Some(pass) = prompt(&format!("{}@{} 的密码: ", host.user, host.hostname)) {
        let ok = session
            .handle
            .authenticate_password(&host.user, &pass)
            .await?
            .success();
        if ok {
            return Ok(session);
        }
    }

    anyhow::bail!("所有认证方式均失败")
}

async fn try_agent_auth(
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
        {
            if res.success() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn expand_tilde(p: &PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    p.clone()
}
