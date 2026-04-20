use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::host::{Host, HostSource};

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
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("sushi")
}

pub fn config_path() -> PathBuf {
    config_dir().join("hosts.toml")
}

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

/// 增量合并：从 ssh config 新导入的主机并入已有主机列表。
/// - 已有 `SshConfig` 主机 id 相同 → 更新字段，保留用户的 tags/description
/// - id 不存在 → 新增
/// - `Manual` 主机永不被动过
/// - ssh config 中删除的 Host → 不从结果中移除
pub fn merge_ssh_config_hosts(existing: Vec<Host>, imported: Vec<Host>) -> Vec<Host> {
    let mut result = existing;
    for new_h in imported {
        if let Some(slot) = result
            .iter_mut()
            .find(|h| h.id == new_h.id && matches!(h.source, HostSource::SshConfig))
        {
            let tags = std::mem::take(&mut slot.tags);
            let description = std::mem::take(&mut slot.description);
            *slot = Host {
                tags,
                description,
                ..new_h
            };
        } else if !result.iter().any(|h| h.id == new_h.id) {
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
    fn merge_adds_new_ssh_config_hosts() {
        let existing = vec![sample_host("m1", HostSource::Manual)];
        let imported = vec![sample_host("s1", HostSource::SshConfig)];
        let merged = merge_ssh_config_hosts(existing, imported);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_updates_existing_ssh_config_host_but_keeps_tags() {
        let mut old = sample_host("s1", HostSource::SshConfig);
        old.tags = vec!["prod".into()];
        old.hostname = "old".into();
        let mut incoming = sample_host("s1", HostSource::SshConfig);
        incoming.hostname = "new".into();

        let merged = merge_ssh_config_hosts(vec![old], vec![incoming]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].hostname, "new");
        assert_eq!(merged[0].tags, vec!["prod".to_string()]);
    }

    #[test]
    fn merge_never_touches_manual_hosts() {
        let existing = vec![sample_host("m1", HostSource::Manual)];
        let merged = merge_ssh_config_hosts(existing.clone(), vec![]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "m1");
    }
}
