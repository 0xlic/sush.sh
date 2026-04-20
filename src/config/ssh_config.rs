use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};

use anyhow::Result;

use super::host::Host;

pub fn import_ssh_config() -> Result<(Vec<Host>, String)> {
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join(".ssh/config");

    if !path.exists() {
        return Ok((Vec::new(), String::new()));
    }

    let content = fs::read_to_string(&path)?;
    let hash = compute_hash(&content);

    // TODO: 使用 ssh2-config 解析 SSH 配置
    let hosts = Vec::new();

    Ok((hosts, hash))
}

fn compute_hash(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
