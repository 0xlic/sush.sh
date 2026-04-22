use crate::config::host::Host;

/// Perform multi-field fuzzy search over hosts and return matching original indices
/// sorted by descending score. An empty query returns 0..hosts.len() in order.
pub fn search(query: &str, hosts: &[Host]) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..hosts.len()).collect();
    }

    use nucleo::{Config, Matcher, Utf32Str};

    let mut matcher = Matcher::new(Config::DEFAULT);
    let needle_owned = query.to_lowercase();
    let mut needle_buf: Vec<char> = Vec::new();
    let needle = Utf32Str::new(&needle_owned, &mut needle_buf);

    let mut scored: Vec<(usize, u16)> = hosts
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let haystack = build_haystack(h).to_lowercase();
            let mut haystack_buf: Vec<char> = Vec::new();
            let hay = Utf32Str::new(&haystack, &mut haystack_buf);
            matcher.fuzzy_match(hay, needle).map(|s| (i, s))
        })
        .collect();

    scored.sort_by_key(|b| std::cmp::Reverse(b.1));
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
    use crate::config::host::{Host, HostSource};
    use std::path::PathBuf;

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

    #[test]
    fn empty_query_returns_all() {
        let hosts = vec![make("a", "x", "u", &[]), make("b", "y", "u", &[])];
        assert_eq!(search("", &hosts), vec![0, 1]);
    }

    #[test]
    fn matches_alias() {
        let hosts = vec![
            make("prod-web", "10.0.0.1", "deploy", &[]),
            make("dev-db", "10.0.0.2", "root", &[]),
        ];
        let result = search("web", &hosts);
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn matches_tag() {
        let hosts = vec![make("a", "x", "u", &["web"]), make("b", "y", "u", &["db"])];
        let result = search("db", &hosts);
        assert!(result.contains(&1));
    }

    #[test]
    fn matches_hostname() {
        let hosts = vec![make("a", "example.com", "u", &[])];
        assert_eq!(search("example", &hosts), vec![0]);
    }
}
