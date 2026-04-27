use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ForwardKind {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardRule {
    pub id: String,
    pub name: String,
    pub kind: ForwardKind,
    pub local_port: u16,
    #[serde(default)]
    pub remote_host: Option<String>,
    #[serde(default)]
    pub remote_port: Option<u16>,
    #[serde(default)]
    pub auto_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostSource {
    SshConfig,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    pub alias: String,
    pub hostname: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub identity_files: Vec<PathBuf>,
    #[serde(default)]
    pub proxy_jump: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: String,
    pub source: HostSource,
    #[serde(default)]
    pub forwards: Vec<ForwardRule>,
}

fn default_port() -> u16 {
    22
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_rule_toml_round_trip() {
        let rule = ForwardRule {
            id: "fwd-1".into(),
            name: "Web".into(),
            kind: ForwardKind::Local,
            local_port: 8080,
            remote_host: Some("localhost".into()),
            remote_port: Some(80),
            auto_start: false,
        };
        let s = toml::to_string(&rule).unwrap();
        let back: ForwardRule = toml::from_str(&s).unwrap();
        assert_eq!(back.id, "fwd-1");
        assert_eq!(back.local_port, 8080);
        assert_eq!(back.kind, ForwardKind::Local);
    }

    #[test]
    fn dynamic_rule_no_remote_fields() {
        let rule = ForwardRule {
            id: "fwd-2".into(),
            name: "SOCKS".into(),
            kind: ForwardKind::Dynamic,
            local_port: 1080,
            remote_host: None,
            remote_port: None,
            auto_start: true,
        };
        let s = toml::to_string(&rule).unwrap();
        let back: ForwardRule = toml::from_str(&s).unwrap();
        assert_eq!(back.kind, ForwardKind::Dynamic);
        assert!(back.remote_host.is_none());
        assert!(back.remote_port.is_none());
    }

    #[test]
    fn host_with_forwards_deserializes() {
        let toml_str = r#"
id = "h1"
alias = "h1"
hostname = "1.2.3.4"
port = 22
user = "admin"
source = "Manual"

[[forwards]]
id = "f1"
name = "Web"
kind = "Local"
local_port = 8080
remote_host = "localhost"
remote_port = 80
auto_start = false
"#;
        let host: Host = toml::from_str(toml_str).unwrap();
        assert_eq!(host.forwards.len(), 1);
        assert_eq!(host.forwards[0].name, "Web");
    }
}
