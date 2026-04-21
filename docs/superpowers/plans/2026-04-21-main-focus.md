# 主界面焦点管理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 主界面搜索框与主机列表之间实现焦点管理，默认焦点在主机列表，Tab 切换焦点，不同焦点下状态栏显示不同快捷键提示。

**Architecture:** 在 `App` 中新增 `MainFocus` 枚举字段，`handle_main_key` 按当前焦点分支处理按键；`HostList` widget 新增 `focused` 字段控制边框样式；`main_view::render` 根据焦点动态传参并切换状态栏提示。

**Tech Stack:** Rust, ratatui 0.30, crossterm 0.29

---

### Task 1: 新增 `MainFocus` 枚举并挂载到 `App`

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: 在 `app.rs` 顶部 `AppMode` 附近新增枚举**

在 `src/app.rs` 第 20 行 `pub enum AppMode` 前插入：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainFocus {
    HostList,
    Search,
}
```

- [ ] **Step 2: 在 `App` 结构体中新增字段**

在 `src/app.rs` `pub struct App` 的 `should_quit` 字段后新增：

```rust
pub main_focus: MainFocus,
```

- [ ] **Step 3: 在 `App::new()` 初始化该字段**

在 `Ok(Self { ... })` 块中 `should_quit: false,` 后面加：

```rust
main_focus: MainFocus::HostList,
```

- [ ] **Step 4: 编译检查**

```bash
cargo check
```

期望：无错误。

- [ ] **Step 5: 提交**

```bash
git add src/app.rs
git commit -m "feat(app): 新增 MainFocus 枚举，App 默认焦点为 HostList"
```

---

### Task 2: `HostList` widget 支持焦点边框样式

**Files:**
- Modify: `src/tui/widgets/host_list.rs`

- [ ] **Step 1: 在 `HostList` 结构体新增 `focused` 字段**

把 `src/tui/widgets/host_list.rs` 第 9-12 行改为：

```rust
pub struct HostList<'a> {
    pub hosts: &'a [Host],
    pub indices: &'a [usize],
    pub focused: bool,
}
```

- [ ] **Step 2: 在 `render` 中根据 `focused` 改变边框颜色**

把第 18 行 `let block = Block::bordered().title(" 主机 ");` 改为：

```rust
use ratatui::style::Color;
let block = if self.focused {
    Block::bordered()
        .title(" 主机 ")
        .border_style(Style::default().fg(Color::Cyan))
} else {
    Block::bordered().title(" 主机 ")
};
```

注意：`Color` 已在文件顶部 use，不需要重复 import，保留现有 use 即可。检查第 1-6 行的 use 语句，`Color` 已在 `use ratatui::style::{Color, Modifier, Style};` 中，把内联的 `use ratatui::style::Color;` 删掉，只写：

```rust
let block = if self.focused {
    Block::bordered()
        .title(" 主机 ")
        .border_style(Style::default().fg(Color::Cyan))
} else {
    Block::bordered().title(" 主机 ")
};
```

- [ ] **Step 3: 修复测试中 `HostList` 构造，补充 `focused` 字段**

在 `host_list.rs` 测试中找到：

```rust
StatefulWidget::render(
    HostList { hosts: &hosts, indices: &indices },
```

改为：

```rust
StatefulWidget::render(
    HostList { hosts: &hosts, indices: &indices, focused: false },
```

- [ ] **Step 4: 运行测试**

```bash
cargo test tui::widgets::host_list
```

期望：`description_renders_when_selected` PASS。

- [ ] **Step 5: 提交**

```bash
git add src/tui/widgets/host_list.rs
git commit -m "feat(host_list): 新增 focused 字段，焦点时边框高亮为青色"
```

---

### Task 3: `handle_main_key` 按焦点分支，更新 `main_view::render`

**Files:**
- Modify: `src/app.rs`
- Modify: `src/tui/views/main_view.rs`

- [ ] **Step 1: 重写 `handle_main_key`**

把 `src/app.rs` 中整个 `fn handle_main_key` 替换为：

```rust
fn handle_main_key(&mut self, k: KeyEvent) {
    match self.main_focus {
        MainFocus::HostList => self.handle_main_key_hostlist(k),
        MainFocus::Search => self.handle_main_key_search(k),
    }
}

fn handle_main_key_hostlist(&mut self, k: KeyEvent) {
    match (k.code, k.modifiers) {
        (KeyCode::Tab, _) | (KeyCode::Char('/'), KeyModifiers::NONE) => {
            self.main_focus = MainFocus::Search;
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            self.trigger_connect = true;
        }
        (KeyCode::Enter, KeyModifiers::SHIFT) => {
            self.trigger_sftp = true;
        }
        (KeyCode::F(2), _) => {
            self.trigger_sftp = true;
        }
        (KeyCode::Up, _) => self.select_previous(),
        (KeyCode::Down, _) => self.select_next(),
        (KeyCode::Char('q'), KeyModifiers::NONE) => {
            self.should_quit = true;
        }
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            self.should_quit = true;
        }
        // n/e/d/? 暂无实现
        _ => {}
    }
}

fn handle_main_key_search(&mut self, k: KeyEvent) {
    match (k.code, k.modifiers) {
        (KeyCode::Tab, _) | (KeyCode::Esc, _) => {
            self.main_focus = MainFocus::HostList;
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            self.trigger_connect = true;
        }
        (KeyCode::Enter, KeyModifiers::SHIFT) => {
            self.trigger_sftp = true;
        }
        (KeyCode::Up, _) => self.select_previous(),
        (KeyCode::Down, _) => self.select_next(),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            self.search_query.clear();
            self.apply_search();
        }
        (KeyCode::Backspace, _) => {
            self.search_query.pop();
            self.apply_search();
        }
        (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
            self.search_query.push(c);
            self.apply_search();
        }
        _ => {}
    }
}
```

- [ ] **Step 2: 编译检查**

```bash
cargo check
```

期望：无错误。

- [ ] **Step 3: 更新 `main_view::render`，传入焦点状态**

把 `src/tui/views/main_view.rs` 的函数签名和实现整体替换为：

```rust
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::ListState;

use crate::app::{App, MainFocus};
use crate::tui::widgets::host_list::HostList;
use crate::tui::widgets::search_input::SearchInput;
use crate::tui::widgets::status_bar::StatusBar;

pub fn render(f: &mut Frame, app: &App, list_state: &mut ListState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    let search_focused = app.main_focus == MainFocus::Search;

    f.render_widget(
        SearchInput {
            query: &app.search_query,
            focused: search_focused,
        },
        chunks[0],
    );

    f.render_stateful_widget(
        HostList {
            hosts: &app.hosts,
            indices: &app.filtered_indices,
            focused: !search_focused,
        },
        chunks[1],
        list_state,
    );

    let hints: &[(&str, &str)] = if search_focused {
        &[
            ("Enter", "SSH"),
            ("S+Enter", "SFTP"),
        ]
    } else {
        &[
            ("/", "搜索"),
            ("Enter", "SSH"),
            ("S+Enter", "SFTP"),
            ("n", "新建"),
            ("e", "编辑"),
            ("d", "删除"),
            ("?", "帮助"),
        ]
    };

    f.render_widget(StatusBar { hints }, chunks[2]);
}
```

- [ ] **Step 4: 编译检查**

```bash
cargo check
```

期望：无错误。

- [ ] **Step 5: Clippy**

```bash
cargo clippy -- -D warnings
```

期望：无警告。

- [ ] **Step 6: 格式化检查**

```bash
cargo fmt --check
```

如有格式问题，执行 `cargo fmt` 修正。

- [ ] **Step 7: 手动运行验证**

```bash
cargo run
```

验证：
- 启动后焦点在主机列表（青色边框），状态栏显示 `/ 搜索  Enter SSH  S+Enter SFTP  n 新建  e 编辑  d 删除  ? 帮助`
- 按 `/` 或 `Tab`：焦点切到搜索框（青色边框消失，搜索框出现光标），状态栏变为 `Enter SSH  S+Enter SFTP`
- 在搜索框输入字符：主机列表实时过滤
- `Tab` 或 `Esc`：焦点回到主机列表
- `Up/Down` 在两种焦点下均可导航列表
- `n/e/d/?` 在主机列表焦点下按下无响应（不崩溃）

- [ ] **Step 8: 提交**

```bash
git add src/app.rs src/tui/views/main_view.rs
git commit -m "feat(tui): 主界面焦点管理，Tab/斜杠切换焦点，状态栏按焦点动态切换"
```
