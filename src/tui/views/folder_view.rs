use std::collections::HashMap;

use crate::config::host::Host;

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    pub children: Vec<String>, // sorted full paths of direct children
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FolderFocus {
    DirA,
    DirB,
    Hosts,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct JumpState {
    pub input: String,
    pub candidates: Vec<String>,
    pub sel: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchState {
    pub scope_path: String,
    pub query: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FolderViewState {
    pub tree: HashMap<String, FolderNode>,
    pub col_a: Vec<String>,
    pub col_b: Vec<String>,
    pub sel_a: usize,
    pub sel_b: usize,
    pub host_sel: usize, // selected index within folder_host_indices
    pub depth: usize,
    pub focus: FolderFocus,
    pub jump: Option<JumpState>,
    pub search: Option<SearchState>,
}

#[allow(dead_code)]
impl FolderViewState {
    pub fn new(hosts: &[Host]) -> Self {
        let tree = build_tree(hosts);
        let col_a = level_1_paths(&tree);
        let col_b = col_a
            .first()
            .and_then(|p| tree.get(p))
            .map(|n| n.children.clone())
            .unwrap_or_default();
        Self {
            tree,
            col_a,
            col_b,
            sel_a: 0,
            sel_b: 0,
            host_sel: 0,
            depth: 0,
            focus: FolderFocus::DirA,
            jump: None,
            search: None,
        }
    }

    /// The path whose hosts should be shown in the Hosts column.
    pub fn focused_path(&self) -> &str {
        match self.focus {
            FolderFocus::DirA => self.col_a.get(self.sel_a).map(|s| s.as_str()).unwrap_or("/"),
            FolderFocus::DirB | FolderFocus::Hosts => {
                if !self.col_b.is_empty() {
                    self.col_b.get(self.sel_b).map(|s| s.as_str()).unwrap_or("/")
                } else {
                    self.col_a.get(self.sel_a).map(|s| s.as_str()).unwrap_or("/")
                }
            }
        }
    }

    /// Recompute col_b from the current col_a selection.
    pub fn update_col_b(&mut self) {
        let selected = self.col_a.get(self.sel_a).cloned().unwrap_or_else(|| "/".into());
        self.col_b = self
            .tree
            .get(&selected)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        self.sel_b = 0;
    }

    /// Jump to a specific path: rebuild col_a as the siblings, col_b as children.
    pub fn jump_to(&mut self, path: &str) {
        let parent = parent_path(path);
        self.col_a = self
            .tree
            .get(&parent)
            .map(|n| n.children.clone())
            .unwrap_or_else(|| level_1_paths(&self.tree));
        self.sel_a = self.col_a.iter().position(|p| p == path).unwrap_or(0);
        self.update_col_b();
        self.focus = FolderFocus::DirA;
        self.jump = None;
    }
}

// ── Tree construction ─────────────────────────────────────────────────────────

pub fn build_tree(hosts: &[Host]) -> HashMap<String, FolderNode> {
    let mut tree: HashMap<String, FolderNode> = HashMap::new();
    tree.entry("/".into()).or_insert_with(|| FolderNode {
        path: "/".into(),
        name: "/".into(),
        children: vec![],
    });

    for host in hosts {
        let path_tags: Vec<String> = host
            .tags
            .iter()
            .filter(|t| t.starts_with('/') && t.len() > 1)
            .cloned()
            .collect();

        for path in path_tags {
            ensure_path(&mut tree, &path);
        }
    }
    tree
}

fn ensure_path(tree: &mut HashMap<String, FolderNode>, path: &str) {
    if tree.contains_key(path) {
        return;
    }
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let name = parts.last().copied().unwrap_or(path).to_string();

    tree.insert(
        path.to_string(),
        FolderNode { path: path.to_string(), name, children: vec![] },
    );

    let parent = parent_path(path);
    ensure_path(tree, &parent);
    if let Some(parent_node) = tree.get_mut(&parent)
        && !parent_node.children.contains(&path.to_string())
    {
        parent_node.children.push(path.to_string());
        parent_node.children.sort();
    }
}

pub fn parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        None | Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
    }
}

/// Level-1 paths: "/" plus all single-component paths like "/prod".
pub fn level_1_paths(tree: &HashMap<String, FolderNode>) -> Vec<String> {
    let mut paths: Vec<String> = tree
        .keys()
        .filter(|p| {
            if *p == "/" {
                return true;
            }
            let trimmed = p.trim_start_matches('/');
            !trimmed.contains('/')
        })
        .cloned()
        .collect();
    paths.sort();
    paths
}

// ── Host filtering ─────────────────────────────────────────────────────────────

/// Returns host indices (into `hosts` slice) that belong to `path` recursively.
/// "/" returns hosts with no path tags at all.
#[allow(dead_code)]
pub fn hosts_in_path(path: &str, hosts: &[Host]) -> Vec<usize> {
    let prefix = if path == "/" { None } else { Some(format!("{path}/")) };
    hosts
        .iter()
        .enumerate()
        .filter(|(_, h)| {
            let path_tags: Vec<&str> = h
                .tags
                .iter()
                .filter(|t| t.starts_with('/'))
                .map(|t| t.as_str())
                .collect();
            if path == "/" {
                path_tags.is_empty()
            } else {
                path_tags.iter().any(|t| {
                    *t == path
                        || prefix
                            .as_ref()
                            .is_some_and(|pfx| t.starts_with(pfx.as_str()))
                })
            }
        })
        .map(|(i, _)| i)
        .collect()
}

// ── Jump candidates ────────────────────────────────────────────────────────────

/// All paths that contain `query` (case-insensitive substring match), sorted.
#[allow(dead_code)]
pub fn jump_candidates(query: &str, tree: &HashMap<String, FolderNode>) -> Vec<String> {
    if query.is_empty() {
        let mut all: Vec<String> = tree.keys().cloned().collect();
        all.sort();
        return all;
    }
    let q = query.to_lowercase();
    let mut matches: Vec<String> = tree
        .keys()
        .filter(|p| p.to_lowercase().contains(&q))
        .cloned()
        .collect();
    matches.sort();
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::host::{Host, HostSource};

    fn host(id: &str, tags: &[&str]) -> Host {
        Host {
            id: id.into(),
            alias: id.into(),
            hostname: id.into(),
            port: 22,
            user: "u".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            description: String::new(),
            source: HostSource::Manual,
        }
    }

    #[test]
    fn build_tree_creates_root_node() {
        let hosts = vec![host("a", &[])];
        let tree = build_tree(&hosts);
        assert!(tree.contains_key("/"));
    }

    #[test]
    fn build_tree_creates_intermediate_nodes() {
        let hosts = vec![host("a", &["/prod/web"])];
        let tree = build_tree(&hosts);
        assert!(tree.contains_key("/prod"));
        assert!(tree.contains_key("/prod/web"));
        let prod = &tree["/prod"];
        assert!(prod.children.contains(&"/prod/web".to_string()));
    }

    #[test]
    fn level_1_paths_includes_root_and_top_dirs() {
        let hosts = vec![host("a", &["/prod/web"]), host("b", &[])];
        let tree = build_tree(&hosts);
        let l1 = level_1_paths(&tree);
        assert!(l1.contains(&"/".to_string()));
        assert!(l1.contains(&"/prod".to_string()));
        assert!(!l1.contains(&"/prod/web".to_string()));
    }

    #[test]
    fn hosts_in_path_root_returns_untagged() {
        let hosts = vec![host("a", &[]), host("b", &["/prod"])];
        let indices = hosts_in_path("/", &hosts);
        assert_eq!(indices, vec![0]);
    }

    #[test]
    fn hosts_in_path_recursive() {
        let hosts = vec![
            host("a", &["/prod"]),
            host("b", &["/prod/web"]),
            host("c", &["/dev"]),
        ];
        let indices = hosts_in_path("/prod", &hosts);
        assert!(indices.contains(&0));
        assert!(indices.contains(&1));
        assert!(!indices.contains(&2));
    }

    #[test]
    fn hosts_in_path_exact_match_only() {
        let hosts = vec![host("a", &["/prod"]), host("b", &["/production"])];
        let indices = hosts_in_path("/prod", &hosts);
        assert_eq!(indices, vec![0]);
    }

    #[test]
    fn jump_candidates_filters_paths() {
        let hosts = vec![host("a", &["/prod/web"]), host("b", &["/dev"])];
        let tree = build_tree(&hosts);
        let candidates = jump_candidates("pro", &tree);
        assert!(candidates.iter().any(|c| c == "/prod" || c == "/prod/web"));
        assert!(!candidates.iter().any(|c| c == "/dev"));
    }
}
