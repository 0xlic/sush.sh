use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ssh2_config::{ParseRule, SshConfig};

use super::host::{Host, HostSource};

pub fn import_ssh_config() -> Result<(Vec<Host>, String)> {
    let path = dirs::home_dir().unwrap_or_default().join(".ssh/config");
    if !path.exists() {
        return Ok((Vec::new(), String::new()));
    }
    parse_ssh_config(&path)
}

pub fn parse_ssh_config(path: &Path) -> Result<(Vec<Host>, String)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read ssh config: {}", path.display()))?;
    let hash = compute_hash(&content);

    let mut reader = BufReader::new(content.as_bytes());
    let config = SshConfig::default()
        .parse(&mut reader, ParseRule::ALLOW_UNKNOWN_FIELDS)
        .map_err(|e| anyhow::anyhow!("failed to parse ssh config: {e}"))?;

    let hosts = config
        .get_hosts()
        .iter()
        .filter_map(host_from_entry)
        .collect();

    Ok((hosts, hash))
}

fn host_from_entry(entry: &ssh2_config::Host) -> Option<Host> {
    // Use the first non-wildcard, non-negated pattern as the alias.
    let alias = entry.pattern.iter().find_map(|c| {
        if c.negated || is_wildcard(&c.pattern) {
            None
        } else {
            Some(c.pattern.clone())
        }
    })?;

    let params = &entry.params;
    let hostname = params.host_name.clone().unwrap_or_else(|| alias.clone());
    let user = params.user.clone().unwrap_or_default();
    let port = params.port.unwrap_or(22);
    let identity_files: Vec<PathBuf> = params.identity_file.clone().unwrap_or_default();
    let proxy_jump = params.proxy_jump.as_ref().and_then(|v| v.first()).cloned();

    Some(Host {
        id: alias.clone(),
        alias,
        hostname,
        port,
        user,
        identity_files,
        proxy_jump,
        tags: Vec::new(),
        description: String::new(),
        source: HostSource::SshConfig,
    })
}

fn is_wildcard(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?')
}

fn compute_hash(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_config(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn parse_single_host() {
        let f = write_config("Host prod-web\n  HostName 10.0.0.1\n  User deploy\n  Port 2222\n");
        let (hosts, _) = parse_ssh_config(f.path()).unwrap();
        assert_eq!(hosts.len(), 1);
        let h = &hosts[0];
        assert_eq!(h.alias, "prod-web");
        assert_eq!(h.hostname, "10.0.0.1");
        assert_eq!(h.user, "deploy");
        assert_eq!(h.port, 2222);
        assert!(matches!(h.source, HostSource::SshConfig));
    }

    #[test]
    fn skip_wildcard_host() {
        let f = write_config(
            "Host *\n  User root\n\nHost prod-*\n  User deploy\n\nHost real-host\n  HostName 1.2.3.4\n",
        );
        let (hosts, _) = parse_ssh_config(f.path()).unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "real-host");
    }

    #[test]
    fn multiple_identity_files() {
        let f = write_config(
            "Host multi\n  HostName x\n  IdentityFile ~/.ssh/id_a\n  IdentityFile ~/.ssh/id_b\n",
        );
        let (hosts, _) = parse_ssh_config(f.path()).unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].identity_files.len(), 2);
    }
}
