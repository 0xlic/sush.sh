# Embedded Terminal Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace SSH takeover mode (LeaveAlternateScreen + raw I/O passthrough) with an embedded terminal emulator, keeping the sush TUI visible during SSH sessions.

**Architecture:** Feed remote PTY output bytes into `alacritty_terminal::Term` via a `vte::ansi::Processor`. A `TerminalView` ratatui widget reads the resulting cell grid every frame and writes characters and colors into the ratatui `Buffer`. The `App` struct holds a `TerminalEmulator` wrapper that owns both the `Term` and the `Processor`.

**Tech Stack:** `alacritty-terminal 0.26`, `ratatui 0.30`, existing `russh 0.60`

> **API note:** `alacritty_terminal`'s internal types (`Color` variants, `Indexed<T>` fields, `Processor` path) must be verified against actual `cargo check` output during implementation. All type paths below are accurate as of 0.26 but adjust if the compiler disagrees.

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `src/ssh/terminal.rs` | `TerminalEmulator` — wraps `Term` + `Processor`, exposes `process()`, `resize()`, `renderable_content()` |
| Create | `src/tui/views/ssh_view.rs` | `render()` layout + `TerminalView` widget + `map_color()` |
| Modify | `src/ssh/mod.rs` | export `terminal` module |
| Modify | `src/tui/views/mod.rs` | export `ssh_view` module |
| Modify | `src/app.rs` | wire `TerminalEmulator`, remove alternate-screen calls, add SSH render arm |
| Modify | `Cargo.toml` | add `alacritty-terminal` dependency |

---

## Task 1: Add alacritty-terminal dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, under `[dependencies]`, add after the `russh-sftp` line:

```toml
alacritty-terminal = "0.26"
```

- [ ] **Step 2: Verify it resolves**

```bash
cargo check
```

Expected: compiles (only new crate downloaded, no code changed yet). If version 0.26 is not found, run `cargo search alacritty-terminal` and use the latest available version.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(deps): add alacritty-terminal for embedded SSH terminal emulator"
```

---

## Task 2: Create TerminalEmulator in `src/ssh/terminal.rs`

**Files:**
- Create: `src/ssh/terminal.rs`
- Modify: `src/ssh/mod.rs`

- [ ] **Step 1: Write failing tests**

Create `src/ssh/terminal.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_correct_dimensions() {
        let em = TerminalEmulator::new(80, 24);
        assert_eq!(em.cols(), 80);
        assert_eq!(em.rows(), 24);
    }

    #[test]
    fn process_ascii_appears_in_grid() {
        let mut em = TerminalEmulator::new(80, 24);
        em.process(b"hi");
        let content = em.renderable_content();
        // First two cells in top-left should be 'h' and 'i'
        let chars: Vec<char> = content
            .display_iter
            .take(2)
            .map(|ic| ic.inner.c)
            .collect();
        assert_eq!(chars, vec!['h', 'i']);
    }

    #[test]
    fn resize_updates_dimensions() {
        let mut em = TerminalEmulator::new(80, 24);
        em.resize(120, 40);
        assert_eq!(em.cols(), 120);
        assert_eq!(em.rows(), 40);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test ssh::terminal
```

Expected: compile error — `TerminalEmulator` not defined yet.

- [ ] **Step 3: Write `TerminalEmulator` implementation**

Replace the contents of `src/ssh/terminal.rs` with the full implementation plus tests:

```rust
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::{Config, RenderableContent};
use alacritty_terminal::vte::ansi::Processor;
use alacritty_terminal::Term;

// Minimal Dimensions implementation for creating/resizing Term.
struct TermSize {
    cols: usize,
    lines: usize,
}

impl alacritty_terminal::grid::Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.cols
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn total_lines(&self) -> usize {
        self.lines
    }
}

// No-op event listener — terminal events (title change, clipboard, etc.)
// are not handled in v0.2.
struct VoidListener;

impl EventListener for VoidListener {
    fn send_event(&self, _: Event) {}
}

pub struct TerminalEmulator {
    term: Term<VoidListener>,
    processor: Processor,
    cols: u16,
    rows: u16,
}

impl TerminalEmulator {
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TermSize {
            cols: cols as usize,
            lines: rows as usize,
        };
        let term = Term::new(Config::default(), &size, VoidListener);
        Self {
            term,
            processor: Processor::new(),
            cols,
            rows,
        }
    }

    /// Feed raw PTY output bytes into the terminal state machine.
    pub fn process(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        let size = TermSize {
            cols: cols as usize,
            lines: rows as usize,
        };
        self.term.resize(size);
    }

    pub fn renderable_content(&self) -> RenderableContent<'_> {
        self.term.renderable_content()
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_correct_dimensions() {
        let em = TerminalEmulator::new(80, 24);
        assert_eq!(em.cols(), 80);
        assert_eq!(em.rows(), 24);
    }

    #[test]
    fn process_ascii_appears_in_grid() {
        let mut em = TerminalEmulator::new(80, 24);
        em.process(b"hi");
        let content = em.renderable_content();
        let chars: Vec<char> = content
            .display_iter
            .take(2)
            .map(|ic| ic.inner.c)
            .collect();
        assert_eq!(chars, vec!['h', 'i']);
    }

    #[test]
    fn resize_updates_dimensions() {
        let mut em = TerminalEmulator::new(80, 24);
        em.resize(120, 40);
        assert_eq!(em.cols(), 120);
        assert_eq!(em.rows(), 40);
    }
}
```

> **Compile-time adjustments:** If `ic.inner` doesn't exist, try `*ic` or `ic.cell`. If `Processor::new()` fails, try `Processor::default()`. If `alacritty_terminal::grid::Dimensions` path is wrong, run `cargo doc --open` and search for `Dimensions`.

- [ ] **Step 4: Export from `src/ssh/mod.rs`**

Open `src/ssh/mod.rs` and add:

```rust
pub mod terminal;
```

- [ ] **Step 5: Run tests**

```bash
cargo test ssh::terminal
```

Expected: all 3 tests pass.

- [ ] **Step 6: Clippy**

```bash
cargo clippy -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add src/ssh/terminal.rs src/ssh/mod.rs
git commit -m "feat(ssh): add TerminalEmulator wrapping alacritty_terminal"
```

---

## Task 3: Create `src/tui/views/ssh_view.rs` with TerminalView widget

**Files:**
- Create: `src/tui/views/ssh_view.rs`
- Modify: `src/tui/views/mod.rs`

- [ ] **Step 1: Write failing color mapping tests**

Create `src/tui/views/ssh_view.rs` with only tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::Color as AColor;

    #[test]
    fn indexed_color_passes_through() {
        assert_eq!(map_color(AColor::Indexed(42)), ratatui::style::Color::Indexed(42));
    }

    #[test]
    fn indexed_zero_maps_correctly() {
        assert_eq!(map_color(AColor::Indexed(0)), ratatui::style::Color::Indexed(0));
    }
}
```

- [ ] **Step 2: Run to verify fails**

```bash
cargo test tui::views::ssh_view
```

Expected: compile error.

- [ ] **Step 3: Write full `ssh_view.rs`**

```rust
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::ssh::terminal::TerminalEmulator;

/// Top-level render for SSH mode: terminal area + status bar.
pub fn render(f: &mut Frame, host_alias: &str, emulator: &TerminalEmulator) {
    let [terminal_area, status_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    f.render_widget(TerminalView { emulator }, terminal_area);
    render_status_bar(f, status_area, host_alias);
}

fn render_status_bar(f: &mut Frame, area: Rect, host_alias: &str) {
    let text = format!(" SSH: {host_alias}    Ctrl-\\:SFTP  Ctrl-D:断开");
    let bar = Paragraph::new(text)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    f.render_widget(bar, area);
}

struct TerminalView<'a> {
    emulator: &'a TerminalEmulator,
}

impl Widget for TerminalView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let content = self.emulator.renderable_content();

        for indexed_cell in content.display_iter {
            let point = indexed_cell.point;
            let cell = &indexed_cell.inner;

            // line.0 is i32; visible cells from display_iter are 0..rows
            let line = point.line.0;
            if line < 0 {
                continue;
            }
            let x = area.x.saturating_add(point.column.0 as u16);
            let y = area.y.saturating_add(line as u16);
            if x >= area.right() || y >= area.bottom() {
                continue;
            }

            let buf_cell = &mut buf[(x, y)];
            buf_cell.set_char(cell.c);
            buf_cell.set_fg(map_color(cell.fg));
            buf_cell.set_bg(map_color(cell.bg));

            let mut modifier = Modifier::empty();
            if cell.flags.contains(Flags::BOLD) {
                modifier |= Modifier::BOLD;
            }
            if cell.flags.contains(Flags::ITALIC) {
                modifier |= Modifier::ITALIC;
            }
            if cell.flags.contains(Flags::UNDERLINE) {
                modifier |= Modifier::UNDERLINED;
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                modifier |= Modifier::CROSSED_OUT;
            }
            if !modifier.is_empty() {
                buf_cell.set_style(Style::default().add_modifier(modifier));
            }
        }

        // Draw cursor by inverting colors at cursor position.
        let cursor = &content.cursor;
        let line = cursor.point.line.0;
        if line >= 0 {
            let x = area.x.saturating_add(cursor.point.column.0 as u16);
            let y = area.y.saturating_add(line as u16);
            if x < area.right() && y < area.bottom() {
                let c = &mut buf[(x, y)];
                let fg = c.fg;
                let bg = c.bg;
                c.set_fg(bg).set_bg(fg);
            }
        }
    }
}

pub fn map_color(color: AColor) -> Color {
    match color {
        AColor::Named(named) => map_named(named),
        AColor::Indexed(idx) => Color::Indexed(idx),
        // alacritty_terminal 0.26 uses Rgb(vte::ansi::Rgb { r, g, b })
        // If the variant is `Spec(Rgb)` instead, adjust the pattern accordingly.
        AColor::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn map_named(named: NamedColor) -> Color {
    match named {
        NamedColor::Black => Color::Black,
        NamedColor::Red => Color::Red,
        NamedColor::Green => Color::Green,
        NamedColor::Yellow => Color::Yellow,
        NamedColor::Blue => Color::Blue,
        NamedColor::Magenta => Color::Magenta,
        NamedColor::Cyan => Color::Cyan,
        NamedColor::White => Color::White,
        NamedColor::BrightBlack => Color::DarkGray,
        NamedColor::BrightRed => Color::LightRed,
        NamedColor::BrightGreen => Color::LightGreen,
        NamedColor::BrightYellow => Color::LightYellow,
        NamedColor::BrightBlue => Color::LightBlue,
        NamedColor::BrightMagenta => Color::LightMagenta,
        NamedColor::BrightCyan => Color::LightCyan,
        NamedColor::BrightWhite => Color::White,
        // Foreground/background defaults — let the terminal decide.
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::Color as AColor;

    #[test]
    fn indexed_color_passes_through() {
        assert_eq!(map_color(AColor::Indexed(42)), ratatui::style::Color::Indexed(42));
    }

    #[test]
    fn indexed_zero_maps_correctly() {
        assert_eq!(map_color(AColor::Indexed(0)), ratatui::style::Color::Indexed(0));
    }
}
```

> **Compile-time adjustments:** If `AColor::Rgb` variant doesn't exist, check for `AColor::Spec(Rgb)` and adjust the match arm. If `indexed_cell.inner` doesn't compile, try `(*indexed_cell).c` or look at what `Indexed<&Cell>` exposes via `cargo doc`.

- [ ] **Step 4: Export from `src/tui/views/mod.rs`**

Add to `src/tui/views/mod.rs`:

```rust
pub mod ssh_view;
```

- [ ] **Step 5: Run tests**

```bash
cargo test tui::views::ssh_view
```

Expected: 2 color mapping tests pass.

- [ ] **Step 6: Clippy**

```bash
cargo clippy -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add src/tui/views/ssh_view.rs src/tui/views/mod.rs
git commit -m "feat(tui): add ssh_view with TerminalView widget and ANSI color mapping"
```

---

## Task 4: Wire TerminalEmulator into `app.rs`

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add imports at top of `src/app.rs`**

Add to the `use` block:

```rust
use crate::ssh::terminal::TerminalEmulator;
use crate::tui::views::ssh_view;
```

Remove (or confirm unused after this task):

```rust
use tokio::io::AsyncWriteExt;  // remove — no longer writing to stdout in SSH mode
```

- [ ] **Step 2: Add field to `App` struct**

In the `App` struct definition, add after `ssh_last_size`:

```rust
pub terminal_emulator: Option<TerminalEmulator>,
```

- [ ] **Step 3: Initialize field in `App::new()`**

In the `Ok(Self { ... })` block inside `App::new()`, add:

```rust
terminal_emulator: None,
```

- [ ] **Step 4: Modify `ssh_connect_and_takeover`**

Replace the entire method body:

```rust
async fn ssh_connect_and_takeover(
    &mut self,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    bus: &mut EventBus,
    host: &Host,
) -> Result<()> {
    let mut session = self.connect_with_prompt(terminal, bus, host).await?;
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    // Reserve 1 row for the status bar.
    let term_rows = rows.saturating_sub(1).max(1);
    session.request_pty(cols, term_rows).await?;

    self.terminal_emulator = Some(TerminalEmulator::new(cols, term_rows));
    self.active_session = Some(session);
    self.current_host_alias = Some(host.alias.clone());
    self.mode = AppMode::Ssh;
    self.ssh_last_size = Some((cols, rows));
    Ok(())
}
```

- [ ] **Step 5: Modify `handle_ssh_channel_msg`**

Replace the `ChannelMsg::Data` and `ChannelMsg::ExtendedData` arms:

```rust
Some(ChannelMsg::Data { ref data }) => {
    if let Some(emulator) = &mut self.terminal_emulator {
        emulator.process(data);
    }
}
Some(ChannelMsg::ExtendedData { ref data, .. }) => {
    // stderr also rendered in the terminal (e.g. shell error messages)
    if let Some(emulator) = &mut self.terminal_emulator {
        emulator.process(data);
    }
}
```

- [ ] **Step 6: Modify `leave_ssh_mode`**

Replace the method body:

```rust
async fn leave_ssh_mode(
    &mut self,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<()> {
    terminal.clear()?;
    self.ssh_last_size = None;
    self.terminal_emulator = None;
    if let Some(session) = self.active_session.take() {
        let _ = session.disconnect().await;
    }
    self.sftp_client = None;
    self.sftp_pane = None;
    self.current_host_alias = None;
    self.mode = AppMode::Main;
    Ok(())
}
```

- [ ] **Step 7: Modify `switch_ssh_to_sftp`**

Remove the `crossterm::execute!(... EnterAlternateScreen)` call. The method body becomes:

```rust
async fn switch_ssh_to_sftp(
    &mut self,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<()> {
    if self.sftp_client.is_none() || self.sftp_pane.is_none() {
        let Some(session) = self.active_session.as_ref() else {
            bail!("SSH session does not exist");
        };
        let client = SftpClient::open(session).await?;
        let home = client.home_dir().await;
        let remote_entries = client.list_dir(&home).await.unwrap_or_default();
        let local_path = std::env::current_dir().unwrap_or_default();
        let local_entries = list_local(&local_path).unwrap_or_default();

        let mut pane = SftpPaneState::new(home);
        pane.remote_entries = remote_entries;
        pane.local_entries = local_entries;
        self.sftp_client = Some(client);
        self.sftp_pane = Some(pane);
    }

    terminal.clear()?;
    self.mode = AppMode::Sftp;
    Ok(())
}
```

- [ ] **Step 8: Modify `resume_ssh_from_sftp`**

Remove the `crossterm::execute!(... LeaveAlternateScreen)` call, and initialize the emulator when a new PTY is created (the path where user went directly to SFTP without SSH first):

```rust
async fn resume_ssh_from_sftp(&mut self) -> Result<()> {
    let Some(session) = self.active_session.as_mut() else {
        self.exit_sftp();
        return Ok(());
    };

    if !session.has_pty() {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let term_rows = rows.saturating_sub(1).max(1);
        session.request_pty(cols, term_rows).await?;
        self.terminal_emulator = Some(TerminalEmulator::new(cols, term_rows));
        self.ssh_last_size = Some((cols, rows));
    }

    self.mode = AppMode::Ssh;
    Ok(())
}
```

- [ ] **Step 9: Modify `sync_ssh_size`**

Replace the method body:

```rust
async fn sync_ssh_size(&mut self) -> Result<()> {
    let Some(session) = self.active_session.as_mut() else {
        return Ok(());
    };
    if !session.has_pty() {
        return Ok(());
    }
    let size = crossterm::terminal::size().unwrap_or((80, 24));
    if self.ssh_last_size != Some(size) {
        let term_rows = size.1.saturating_sub(1).max(1);
        session.resize_pty(size.0, term_rows).await?;
        if let Some(emulator) = &mut self.terminal_emulator {
            emulator.resize(size.0, term_rows);
        }
        self.ssh_last_size = Some(size);
    }
    Ok(())
}
```

- [ ] **Step 10: Modify `render()`**

Replace the entire `render` method:

```rust
fn render(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let mut list_state = std::mem::take(&mut self.list_state);
    terminal.draw(|f| match self.mode {
        AppMode::Main => {
            main_view::render(f, self, &mut list_state);
            if let Some(pwd) = &self.pwd_dialog {
                pwd.dialog.render(f);
            }
        }
        AppMode::Sftp => {
            if let Some(pane) = &mut self.sftp_pane {
                let alias = self.current_host_alias.as_deref().unwrap_or("");
                let transfer_info =
                    self.active_transfer.as_ref().map(|t| (t.verb, &t.progress));
                sftp_view::render(f, alias, pane, transfer_info);
            }
            if let Some(pwd) = &self.pwd_dialog {
                pwd.dialog.render(f);
            }
        }
        AppMode::Ssh => {
            if let Some(emulator) = &self.terminal_emulator {
                let alias = self.current_host_alias.as_deref().unwrap_or("");
                ssh_view::render(f, alias, emulator);
            }
        }
    })?;
    self.list_state = list_state;
    Ok(())
}
```

- [ ] **Step 11: Remove `#[allow(dead_code)]` from `AppMode::Ssh`**

In the `AppMode` enum definition, remove the attribute:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Main,
    Ssh,   // was: #[allow(dead_code)] Ssh — now actively used in render()
    Sftp,
}
```

- [ ] **Step 12: Update test helper `app_with` in `#[cfg(test)]` block**

In the `app_with` function at the bottom of `app.rs`, add `terminal_emulator: None` to the `App { ... }` struct literal.

- [ ] **Step 13: cargo check**

```bash
cargo check
```

Fix remaining compile errors:
- If `use tokio::io::AsyncWriteExt` is now unused, remove it from the import
- If there are unused `crossterm::execute!` calls elsewhere, remove them

- [ ] **Step 14: cargo test**

```bash
cargo test
```

Expected: all existing tests pass.

- [ ] **Step 15: cargo clippy**

```bash
cargo clippy -- -D warnings
```

- [ ] **Step 16: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): replace SSH takeover mode with embedded terminal emulator"
```

---

## Task 5: Manual verification

- [ ] **Step 1: Build and run**

```bash
cargo run
```

- [ ] **Step 2: Connect to an SSH host via Enter**

Verify:
- Status bar at bottom shows `SSH: <hostname>  Ctrl-\:SFTP  Ctrl-D:断开`
- Terminal output renders in the area above the status bar
- Commands like `ls`, `echo hello`, `pwd` display output correctly

- [ ] **Step 3: Test Ctrl-\ switch to SFTP**

Press `Ctrl-\` during an SSH session. Verify SFTP panel opens without screen flicker.

- [ ] **Step 4: Test switch back to SSH**

Press `Ctrl-\` from SFTP. Verify SSH session resumes and previous output is still in the emulator grid.

- [ ] **Step 5: Test clean exit**

Type `exit` or press `Ctrl-D`. Verify you return to the main host list.

- [ ] **Step 6: Test window resize**

Resize the terminal window during an SSH session. Verify remote shell adapts (run `tput cols` to confirm the remote sees the new width minus the status bar row).

- [ ] **Step 7: Final commit if fixups were needed during verification**

```bash
git add src/
git commit -m "fix(ssh): correct embedded terminal rendering edge cases"
```
