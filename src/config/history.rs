use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize, Default)]
struct HistoryStore {
    #[serde(default)]
    connections: HashMap<String, DateTime<Utc>>,
}

#[allow(dead_code)]
pub struct ConnectionHistory {
    connections: HashMap<String, DateTime<Utc>>,
    path: PathBuf,
}

#[allow(dead_code)]
impl ConnectionHistory {
    pub fn load(path: PathBuf) -> Self {
        let connections = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| toml::from_str::<HistoryStore>(&s).ok())
                .map(|s| s.connections)
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        Self { connections, path }
    }

    pub fn record(&mut self, host_id: &str) {
        self.connections.insert(host_id.to_string(), Utc::now());
        self.persist();
    }

    pub fn last_connected(&self, host_id: &str) -> Option<&DateTime<Utc>> {
        self.connections.get(host_id)
    }

    pub fn days_since(&self, host_id: &str) -> Option<i64> {
        self.connections
            .get(host_id)
            .map(|t| (Utc::now() - *t).num_days())
    }

    fn persist(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let store = HistoryStore {
            connections: self.connections.clone(),
        };
        if let Ok(content) = toml::to_string_pretty(&store) {
            let _ = fs::write(&self.path, content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn empty_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let h = ConnectionHistory::load(dir.path().join("history.toml"));
        assert!(h.last_connected("any").is_none());
    }

    #[test]
    fn record_and_retrieve() {
        let dir = TempDir::new().unwrap();
        let mut h = ConnectionHistory::load(dir.path().join("history.toml"));
        h.record("web-01");
        assert!(h.last_connected("web-01").is_some());
    }

    #[test]
    fn persists_and_reloads() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("history.toml");
        let mut h = ConnectionHistory::load(path.clone());
        h.record("db-01");
        drop(h);
        let h2 = ConnectionHistory::load(path);
        assert!(h2.last_connected("db-01").is_some());
    }

    #[test]
    fn days_since_returns_zero_for_just_recorded() {
        let dir = TempDir::new().unwrap();
        let mut h = ConnectionHistory::load(dir.path().join("history.toml"));
        h.record("srv");
        assert_eq!(h.days_since("srv"), Some(0));
    }

    #[test]
    fn days_since_returns_none_for_unknown() {
        let dir = TempDir::new().unwrap();
        let h = ConnectionHistory::load(dir.path().join("history.toml"));
        assert_eq!(h.days_since("unknown"), None);
    }
}
