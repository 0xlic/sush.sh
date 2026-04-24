use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::host::Host;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct HostStore {
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default)]
    pub hosts: Vec<Host>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Metadata {
    #[serde(default)]
    pub ssh_config_hash: String,
    #[serde(default)]
    pub import_prompted: bool,
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".config")
        .join("sush")
}

pub fn config_path() -> PathBuf {
    config_dir().join("hosts.toml")
}

pub fn load_store(path: &Path) -> Result<HostStore> {
    if !path.exists() {
        return Ok(HostStore::default());
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    toml::from_str(&content).context("failed to parse config")
}

pub fn load_from(path: &Path) -> Result<(Vec<Host>, String)> {
    let store = load_store(path)?;
    Ok((store.hosts, store.metadata.ssh_config_hash))
}

#[allow(dead_code)]
pub fn load_hosts() -> Result<Vec<Host>> {
    let (hosts, _) = load_from(&config_path())?;
    Ok(hosts)
}

#[allow(dead_code)]
pub fn save_hosts(hosts: &[Host], ssh_config_hash: &str) -> Result<()> {
    save_to(&config_path(), hosts, ssh_config_hash, false)
}

pub fn save_to(
    path: &Path,
    hosts: &[Host],
    ssh_config_hash: &str,
    import_prompted: bool,
) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let store = HostStore {
        metadata: Metadata {
            ssh_config_hash: ssh_config_hash.to_string(),
            import_prompted,
        },
        hosts: hosts.to_vec(),
    };
    let content = toml::to_string_pretty(&store)?;
    fs::write(path, content)?;
    Ok(())
}

/// Merge with the persisted sush config taking precedence over SSH config.
/// - Existing hosts from sush (any source) stay unchanged
/// - Hosts present only in SSH config are imported as new entries
/// - Hosts removed from SSH config are not removed from sush
pub fn merge_ssh_config_hosts(existing: Vec<Host>, imported: Vec<Host>) -> Vec<Host> {
    let mut result = existing;
    for new_h in imported {
        if !result.iter().any(|h| h.id == new_h.id) {
            result.push(new_h);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::host::{Host, HostSource};
    use tempfile::TempDir;

    fn sample_host(id: &str, source: HostSource) -> Host {
        Host {
            id: id.into(),
            alias: id.into(),
            hostname: "1.1.1.1".into(),
            port: 22,
            user: "u".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: vec![],
            description: String::new(),
            source,
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hosts.toml");
        let hosts = vec![sample_host("a", HostSource::Manual)];
        save_to(&path, &hosts, "hash1", false).unwrap();
        let (loaded, hash) = load_from(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "a");
        assert_eq!(hash, "hash1");
    }

    #[test]
    fn import_prompted_roundtrips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hosts.toml");
        save_to(&path, &[], "", true).unwrap();
        let store = load_store(&path).unwrap();
        assert!(store.metadata.import_prompted);
    }

    #[test]
    fn import_prompted_defaults_to_false_on_old_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hosts.toml");
        std::fs::write(&path, "[metadata]\nssh_config_hash = \"abc\"\n").unwrap();
        let store = load_store(&path).unwrap();
        assert!(!store.metadata.import_prompted);
    }

    #[test]
    fn merge_adds_new_hosts_from_ssh_config() {
        // Import hosts that do not already exist in sush.
        let existing = vec![sample_host("m1", HostSource::Manual)];
        let imported = vec![sample_host("s1", HostSource::SshConfig)];
        let merged = merge_ssh_config_hosts(existing, imported);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_sush_config_takes_precedence() {
        // Existing sush data is not overwritten by SSH config changes.
        let mut existing = sample_host("s1", HostSource::SshConfig);
        existing.hostname = "sush-managed".into();
        existing.description = "my description".into();

        let mut incoming = sample_host("s1", HostSource::SshConfig);
        incoming.hostname = "ssh-config-new".into();

        let merged = merge_ssh_config_hosts(vec![existing], vec![incoming]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].hostname, "sush-managed");
        assert_eq!(merged[0].description, "my description");
    }

    #[test]
    fn merge_does_not_remove_hosts_deleted_from_ssh_config() {
        // Hosts deleted from SSH config remain in sush.
        let existing = vec![sample_host("m1", HostSource::Manual)];
        let merged = merge_ssh_config_hosts(existing.clone(), vec![]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "m1");
    }
}
