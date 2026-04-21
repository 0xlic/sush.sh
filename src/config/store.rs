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

#[allow(dead_code)]
pub fn load_hosts() -> Result<Vec<Host>> {
    let (hosts, _) = load_from(&config_path())?;
    Ok(hosts)
}

pub fn load_from(path: &Path) -> Result<(Vec<Host>, String)> {
    if !path.exists() {
        return Ok((Vec::new(), String::new()));
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("读取配置失败: {}", path.display()))?;
    let store: HostStore = toml::from_str(&content).context("解析配置失败")?;
    Ok((store.hosts, store.metadata.ssh_config_hash))
}

#[allow(dead_code)]
pub fn save_hosts(hosts: &[Host], ssh_config_hash: &str) -> Result<()> {
    save_to(&config_path(), hosts, ssh_config_hash)
}

pub fn save_to(path: &Path, hosts: &[Host], ssh_config_hash: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let store = HostStore {
        metadata: Metadata {
            ssh_config_hash: ssh_config_hash.to_string(),
        },
        hosts: hosts.to_vec(),
    };
    let content = toml::to_string_pretty(&store)?;
    fs::write(path, content)?;
    Ok(())
}

/// 合并：以 sush 配置为主，SSH config 只补充 sush 中不存在的主机。
/// - sush 中已有的主机（任意来源）→ 保持原样，SSH config 的变化不影响它
/// - SSH config 中有、sush 中没有 → 导入为新条目
/// - SSH config 中删除的主机 → 不从 sush 中移除
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
        save_to(&path, &hosts, "hash1").unwrap();
        let (loaded, hash) = load_from(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "a");
        assert_eq!(hash, "hash1");
    }

    #[test]
    fn merge_adds_new_hosts_from_ssh_config() {
        // sush 中没有的主机，从 SSH config 导入
        let existing = vec![sample_host("m1", HostSource::Manual)];
        let imported = vec![sample_host("s1", HostSource::SshConfig)];
        let merged = merge_ssh_config_hosts(existing, imported);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_sush_config_takes_precedence() {
        // sush 中已有的主机，SSH config 的字段变化不覆盖它
        let mut existing = sample_host("s1", HostSource::SshConfig);
        existing.hostname = "sush-managed".into();
        existing.description = "我的描述".into();

        let mut incoming = sample_host("s1", HostSource::SshConfig);
        incoming.hostname = "ssh-config-new".into();

        let merged = merge_ssh_config_hosts(vec![existing], vec![incoming]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].hostname, "sush-managed");
        assert_eq!(merged[0].description, "我的描述");
    }

    #[test]
    fn merge_does_not_remove_hosts_deleted_from_ssh_config() {
        // SSH config 中删掉的主机，sush 中仍保留
        let existing = vec![sample_host("m1", HostSource::Manual)];
        let merged = merge_ssh_config_hosts(existing.clone(), vec![]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "m1");
    }
}
