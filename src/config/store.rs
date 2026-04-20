use std::fs;
use std::path::PathBuf;

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
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("sushi")
}

pub fn config_path() -> PathBuf {
    config_dir().join("hosts.toml")
}

pub fn load_hosts() -> Result<Vec<Host>> {
    let path = config_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    let store: HostStore =
        toml::from_str(&content).with_context(|| "解析配置文件失败")?;
    Ok(store.hosts)
}

pub fn save_hosts(hosts: &[Host], ssh_config_hash: &str) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let store = HostStore {
        metadata: Metadata {
            ssh_config_hash: ssh_config_hash.to_string(),
        },
        hosts: hosts.to_vec(),
    };
    let content = toml::to_string_pretty(&store)?;
    fs::write(config_path(), content)?;
    Ok(())
}
