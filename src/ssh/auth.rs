use anyhow::Result;
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;

use super::session::ClientHandler;

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
