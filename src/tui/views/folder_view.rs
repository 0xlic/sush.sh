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
            FolderFocus::DirA => self
                .col_a
                .get(self.sel_a)
                .map(|s| s.as_str())
                .unwrap_or("/"),
            FolderFocus::DirB | FolderFocus::Hosts => {
                if !self.col_b.is_empty() {
                    self.col_b
                        .get(self.sel_b)
                        .map(|s| s.as_str())
                        .unwrap_or("/")
                } else {
                    self.col_a
                        .get(self.sel_a)
                        .map(|s| s.as_str())
                        .unwrap_or("/")
                }
            }
        }
    }

    pub fn selected_path(&self) -> &str {
        self.col_a
            .get(self.sel_a)
            .map(|s| s.as_str())
            .unwrap_or("/")
    }

    /// Recompute col_b from the current col_a selection.
    pub fn update_col_b(&mut self) {
        let selected = self
            .col_a
            .get(self.sel_a)
            .cloned()
            .unwrap_or_else(|| "/".into());
        self.col_b = self
            .tree
            .get(&selected)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        self.sel_b = 0;
    }

    /// Jump to a specific path: rebuild col_a as the siblings, col_b as children.
    pub fn jump_to(&mut self, path: &str) {
        if path == "/" {
            self.col_a = level_1_paths(&self.tree);
            self.sel_a = self.col_a.iter().position(|p| p == "/").unwrap_or(0);
            self.update_col_b();
            self.depth = 0;
            self.focus = FolderFocus::DirA;
            self.jump = None;
            return;
        }

        let parent = parent_path(path);
        self.col_a = self
            .tree
            .get(&parent)
            .map(|n| n.children.clone())
            .unwrap_or_else(|| level_1_paths(&self.tree));
        self.sel_a = self.col_a.iter().position(|p| p == path).unwrap_or(0);
        self.update_col_b();
        self.depth = path_depth(path).saturating_sub(1);
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
        FolderNode {
            path: path.to_string(),
            name,
            children: vec![],
        },
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

fn path_depth(path: &str) -> usize {
    if path == "/" {
        0
    } else {
        path.trim_start_matches('/').split('/').count()
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
    let prefix = if path == "/" {
        None
    } else {
        Some(format!("{path}/"))
    };
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

// ── Rendering ─────────────────────────────────────────────────────────────────

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};

use crate::tui::widgets::host_list::HostList;
use crate::tui::widgets::status_bar::StatusBar;

#[allow(dead_code)]
pub fn render(
    f: &mut Frame,
    state: &FolderViewState,
    hosts: &[Host],
    host_indices: &[usize],
    probe: Option<Option<bool>>,
) {
    let [content_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());

    let [col_a_area, col_b_area, hosts_area] = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(60),
    ])
    .areas(content_area);

    render_dir_col(
        f,
        col_a_area,
        &state.col_a,
        state.sel_a,
        state.focus == FolderFocus::DirA,
        &state.tree,
    );
    render_dir_col(
        f,
        col_b_area,
        &state.col_b,
        state.sel_b,
        state.focus == FolderFocus::DirB,
        &state.tree,
    );

    let mut host_list_state = ListState::default();
    if !host_indices.is_empty() {
        host_list_state.select(Some(state.host_sel.min(host_indices.len() - 1)));
    }
    f.render_stateful_widget(
        HostList {
            hosts,
            indices: host_indices,
            focused: state.focus == FolderFocus::Hosts,
            show_selection: state.focus == FolderFocus::Hosts,
            probe,
            status_msg: None,
        },
        hosts_area,
        &mut host_list_state,
    );

    f.render_widget(
        StatusBar {
            hints: &[
                ("j", "Jump"),
                ("/", "Search"),
                ("Tab", "Switch"),
                ("Enter", "Enter dir"),
                ("ESC", "Exit"),
            ],
        },
        status_area,
    );

    // Overlays drawn last (on top)
    if let Some(jump) = &state.jump {
        render_jump_floater(f, jump);
    }
    if let Some(search) = &state.search {
        render_search_bar(f, search);
    }
}

fn render_dir_col(
    f: &mut Frame,
    area: Rect,
    paths: &[String],
    sel: usize,
    focused: bool,
    tree: &HashMap<String, FolderNode>,
) {
    let block = if focused {
        Block::bordered().border_style(Style::default().fg(Color::Cyan))
    } else {
        Block::bordered()
    };

    if paths.is_empty() {
        f.render_widget(block, area);
        return;
    }

    let items: Vec<ListItem> = paths
        .iter()
        .map(|p| {
            let node = tree.get(p);
            let has_children = node.is_some_and(|n| !n.children.is_empty());
            let name = node.map(|n| n.name.as_str()).unwrap_or(p.as_str());
            let indicator = if has_children { "▶ " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::raw(indicator),
                Span::styled(name, Style::default().fg(Color::Cyan)),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(sel));

    f.render_stateful_widget(
        List::new(items).block(block).highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .fg(Color::Yellow),
        ),
        area,
        &mut list_state,
    );
}

fn render_jump_floater(f: &mut Frame, jump: &JumpState) {
    let area = centered_rect(50, f.area());
    f.render_widget(Clear, area);

    let block = Block::bordered()
        .title(" Jump to ")
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [input_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    f.render_widget(
        Paragraph::new(format!("path:{}{}", jump.input, "█")),
        input_area,
    );

    let items: Vec<ListItem> = jump
        .candidates
        .iter()
        .map(|c| ListItem::new(c.as_str()))
        .collect();
    let mut list_state = ListState::default();
    list_state.select(if jump.candidates.is_empty() {
        None
    } else {
        Some(jump.sel)
    });
    f.render_stateful_widget(
        List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        list_area,
        &mut list_state,
    );
}

fn render_search_bar(f: &mut Frame, search: &SearchState) {
    let area = f.area();
    // Show inline at top of screen (y=0)
    let search_area = Rect {
        x: area.x + 1,
        y: area.y,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("path:{} > ", search.scope_path),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(search.query.as_str()),
            Span::styled("█", Style::default().fg(Color::Cyan)),
        ])),
        search_area,
    );
}

fn centered_rect(percent_x: u16, r: Rect) -> Rect {
    let height = 12u16.min(r.height);
    let y = r.y + r.height.saturating_sub(height) / 2;
    let width = r.width * percent_x / 100;
    let x = r.x + r.width.saturating_sub(width) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
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
