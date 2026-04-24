# 设计文档：嵌入式终端模式（v0.2）

## 背景与动机

v0.1 的 SSH 接管模式通过 `LeaveAlternateScreen` 让 sush TUI 完全消失，将 stdin/stdout 直接透传给远程 PTY。这种方式实现简单，但带来两个根本限制：

1. **SSH 会话期间 sush UI 不可见**，无法显示状态栏、连接信息或操作提示
2. **无法感知终端输出内容**，阻碍会话录制、搜索回滚等后续功能

v0.2 用嵌入式终端模拟器模式替换接管模式：引入 `alacritty_terminal` 作为 VT100/xterm 状态机，将远程 PTY 输出渲染为 ratatui widget，TUI 界面在 SSH 连接期间始终保持可见。

## 用户可见变化

- SSH 连接期间，状态栏持续显示主机名与快捷键提示
- 终端内容渲染在 TUI 界面内，而非占满整个终端
- 功能行为与 v0.1 一致：`Ctrl-\` 切换 SFTP，`exit`/`Ctrl-D` 返回主界面

## 架构设计

### 总体思路

```
旧（v0.1 接管模式）          新（v0.2 嵌入式终端模式）
─────────────────────        ──────────────────────────────
SSH 输出 → stdout 直写       SSH 输出 → alacritty_terminal::Term
LeaveAlternateScreen         TUI 全程运行（alternate screen 不退出）
sush UI 不可见               TerminalView widget 渲染 cell 网格
                             状态栏始终可见
```

### SSH 模式布局

```
┌─ sush ─ SSH: prod-web-01 ──────────────────────┐
│                                                 │
│  $ systemctl status nginx                       │
│  ● nginx.service - A high performance web...    │
│    Active: active (running) since ...           │
│                                                 │
│  $ _                                            │
│                                                 │
│                                                 │
│  Ctrl-\:SFTP  Ctrl-D:断开                       │  ← 状态栏
└─────────────────────────────────────────────────┘
```

### 数据流

```
键盘 stdin (raw bytes)
  │
  ▼
EventBus → handle_ssh_input()
  ├── Ctrl-\ (0x1c) → trigger_ssh_to_sftp（不透传）
  └── 其他字节 → session.write_input() → russh channel → 远程 PTY
                                                              │
                                                    ChannelMsg::Data
                                                              │
                                                              ▼
                                                  TerminalEmulator::process(bytes)
                                                  （更新 alacritty_terminal::Term cell 网格）
                                                              │
                                                              ▼
                                                       ratatui 渲染帧
                                                    ssh_view::render()
                                                    └── TerminalView widget
                                                          读取 Term::grid()
                                                          → 写入 ratatui Buffer
```

## 新增模块

### `ssh/terminal.rs` — 终端状态机封装

封装 `alacritty_terminal::Term`，对外暴露简洁接口。以下为意图接口，具体方法签名在实现时对齐 crate 实际 API：

```rust
pub struct TerminalEmulator {
    term: Term<TermEventProxy>,
    parser: vte::Parser,
}

impl TerminalEmulator {
    pub fn new(cols: u16, rows: u16) -> Self { ... }
    pub fn process(&mut self, bytes: &[u8]) { ... }  // 喂入 SSH 输出字节
    pub fn resize(&mut self, cols: u16, rows: u16) { ... }
    pub fn grid(&self) -> &Grid<Cell> { ... }        // 供 widget 读取 cell 网格
}

// alacritty_terminal 要求的事件代理（v0.2 阶段空实现，后续按需扩展）
struct TermEventProxy;
impl EventListener for TermEventProxy {
    fn send_event(&self, _: Event) {}
}
```

### `tui/views/ssh_view.rs` — SSH 终端视图

负责 SSH 模式的整体布局（终端区域 + 状态栏）和 `TerminalView` widget 实现：

```rust
// 布局：终端区域占满可用高度，底部留一行给状态栏
pub fn render(f: &mut Frame, host_alias: &str, emulator: &TerminalEmulator) {
    let [terminal_area, status_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
    ]).areas(f.area());

    f.render_widget(TerminalView::new(emulator), terminal_area);
    render_status_bar(f, status_area, host_alias);
}

// TerminalView：将 Term::grid() 渲染到 ratatui Buffer
// 以下为意图代码，具体坐标访问方式在实现时对齐 alacritty_terminal API
struct TerminalView<'a> { emulator: &'a TerminalEmulator }

impl Widget for TerminalView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let grid = self.emulator.grid();
        for cell in grid.display_iter() {
            let x = area.x + cell.point.column as u16;
            let y = area.y + cell.point.line as u16;
            if x >= area.right() || y >= area.bottom() { continue; }
            let buf_cell = &mut buf[(x, y)];
            buf_cell.set_char(cell.c);
            buf_cell.set_fg(map_color(cell.fg));
            buf_cell.set_bg(map_color(cell.bg));
            buf_cell.set_style(map_flags(cell.flags));  // bold, italic, underline 等
        }
    }
}
```

### 颜色映射

```rust
fn map_color(c: alacritty_terminal::vte::ansi::Color) -> ratatui::style::Color {
    match c {
        Color::Named(n)    => map_named(n),         // Named → ratatui 具名颜色
        Color::Indexed(n)  => Color::Indexed(n),    // 256 色直接对应
        Color::Rgb(r,g,b)  => Color::Rgb(r,g,b),   // TrueColor 直接对应
    }
}
```

## 对现有代码的改动

### `app.rs`

| 位置 | 变更 |
|------|------|
| `App` 结构体 | 新增 `terminal_emulator: Option<TerminalEmulator>` |
| `ssh_connect_and_takeover` | 移除 `LeaveAlternateScreen`；初始化 `TerminalEmulator` |
| `handle_ssh_channel_msg` | `ChannelMsg::Data` → 喂给 `TerminalEmulator::process()`，不再写 stdout |
| `leave_ssh_mode` | 移除 `EnterAlternateScreen`；清空 `terminal_emulator` |
| `switch_ssh_to_sftp` | 移除 `EnterAlternateScreen` |
| `resume_ssh_from_sftp` | 移除 `LeaveAlternateScreen` |
| `render()` | SSH 模式不再跳过渲染；调用 `ssh_view::render()` |
| `sync_ssh_size` | resize 时同步调用 `TerminalEmulator::resize()` |

### `ssh/session.rs`

无需改动。`write_input` 和 `wait_channel_msg` 接口不变。

### `Cargo.toml`

新增依赖：
```toml
alacritty_terminal = "x.y"   # 版本在实现时确认
```

## 尺寸同步

`TerminalView` 渲染时占用的 `area` 决定终端的有效尺寸。当 ratatui 检测到终端 resize 时：

1. `sync_ssh_size()` 计算新的 `(cols, rows)`（需减去状态栏 1 行）
2. 调用 `TerminalEmulator::resize(cols, rows - 1)`
3. 调用 `session.resize_pty(cols, rows - 1)`

## 已知限制与后续扩展

- `alacritty_terminal` 的 `EventListener` 携带鼠标报告、标题变更等事件，v0.2 阶段以空实现处理，后续按需接入
- 终端颜色主题（Named color 的实际 RGB 值）v0.2 使用 ratatui 默认，后续可支持自定义
- 会话录制功能在此架构基础上于后续版本实现（此时可直接读取 `TerminalEmulator` 的状态）
