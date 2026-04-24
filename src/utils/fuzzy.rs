use std::cmp::Reverse;

use crate::config::history::ConnectionHistory;
use crate::config::host::Host;

/// Fuzzy search with recency boost.
/// Empty query: sort by last_connected DESC then hostname ASC.
/// Non-empty query: nucleo score + recency bonus, sort by combined score DESC.
pub fn search(query: &str, hosts: &[Host], history: &ConnectionHistory) -> Vec<usize> {
    if query.trim().is_empty() {
        let mut indices: Vec<usize> = (0..hosts.len()).collect();
        indices.sort_by(|&a, &b| {
            let ta = history.last_connected(&hosts[a].id);
            let tb = history.last_connected(&hosts[b].id);
            match (ta, tb) {
                (Some(ta), Some(tb)) => tb.cmp(ta),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => hosts[a].hostname.cmp(&hosts[b].hostname),
            }
        });
        return indices;
    }

    use nucleo::{Config, Matcher, Utf32Str};

    let mut matcher = Matcher::new(Config::DEFAULT);
    let needle_owned = query.to_lowercase();
    let mut needle_buf: Vec<char> = Vec::new();
    let needle = Utf32Str::new(&needle_owned, &mut needle_buf);

    let mut scored: Vec<(usize, u32)> = hosts
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let haystack = build_haystack(h).to_lowercase();
            let mut haystack_buf: Vec<char> = Vec::new();
            let hay = Utf32Str::new(&haystack, &mut haystack_buf);
            matcher.fuzzy_match(hay, needle).map(|s| {
                let bonus: u32 = match history.days_since(&h.id) {
                    Some(0..=7) => 30,
                    Some(8..=30) => 15,
                    Some(_) => 5,
                    None => 0,
                };
                (i, s as u32 + bonus)
            })
        })
        .collect();

    scored.sort_by_key(|&(_, score)| Reverse(score));
    scored.into_iter().map(|(i, _)| i).collect()
}

fn build_haystack(h: &Host) -> String {
    let mut s = String::new();
    s.push_str(&h.alias);
    s.push(' ');
    s.push_str(&h.hostname);
    s.push(' ');
    s.push_str(&h.user);
    s.push(' ');
    s.push_str(&h.tags.join(" "));
    s.push(' ');
    s.push_str(&h.description);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::history::ConnectionHistory;
    use crate::config::host::{Host, HostSource};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make(id: &str, hostname: &str, user: &str, tags: &[&str]) -> Host {
        Host {
            id: id.into(),
            alias: id.into(),
            hostname: hostname.into(),
            port: 22,
            user: user.into(),
            identity_files: Vec::<PathBuf>::new(),
            proxy_jump: None,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            description: String::new(),
            source: HostSource::Manual,
        }
    }

    fn empty_history() -> ConnectionHistory {
        let dir = TempDir::new().unwrap();
        ConnectionHistory::load(dir.path().join("h.toml"))
    }

    fn history_with(host_id: &str) -> (ConnectionHistory, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut h = ConnectionHistory::load(dir.path().join("h.toml"));
        h.record(host_id);
        (h, dir)
    }

    #[test]
    fn empty_query_returns_all_sorted_by_hostname_when_no_history() {
        let hosts = vec![
            make("z", "z.example.com", "u", &[]),
            make("a", "a.example.com", "u", &[]),
        ];
        let history = empty_history();
        let result = search("", &hosts, &history);
        assert_eq!(result[0], 1); // a.example.com
        assert_eq!(result[1], 0); // z.example.com
    }

    #[test]
    fn empty_query_sorts_by_history_desc_then_hostname_asc() {
        let hosts = vec![
            make("b-host", "b.example.com", "u", &[]),
            make("a-host", "a.example.com", "u", &[]),
        ];
        let (history, _dir) = history_with("b-host");
        let result = search("", &hosts, &history);
        assert_eq!(result[0], 0); // b-host has history
        assert_eq!(result[1], 1);
    }

    #[test]
    fn matches_alias() {
        let hosts = vec![
            make("prod-web", "10.0.0.1", "deploy", &[]),
            make("dev-db", "10.0.0.2", "root", &[]),
        ];
        let history = empty_history();
        let result = search("web", &hosts, &history);
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn matches_tag() {
        let hosts = vec![
            make("a", "x", "u", &["web"]),
            make("b", "y", "u", &["db"]),
        ];
        let history = empty_history();
        let result = search("db", &hosts, &history);
        assert!(result.contains(&1));
    }

    #[test]
    fn matches_hostname() {
        let hosts = vec![make("a", "example.com", "u", &[])];
        let history = empty_history();
        assert_eq!(search("example", &hosts, &history), vec![0]);
    }

    #[test]
    fn search_boosts_recently_connected_host() {
        let hosts = vec![
            make("prod-web", "10.0.0.1", "deploy", &[]),
            make("prod-db", "10.0.0.2", "deploy", &[]),
        ];
        let (history, _dir) = history_with("prod-db");
        let result = search("prod", &hosts, &history);
        assert_eq!(result[0], 1); // prod-db boosted
    }
}
