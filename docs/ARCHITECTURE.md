# sush.sh (sush) 系统架构设计文档

## 技术栈

> 版本锁定于 2026-04-20，基于 crates.io 最新稳定版选型。

| 层级 | 选择 | 版本 | 说明 |
|------|------|------|------|
| 语言 | Rust | 1.95 (edition 2024) | mise 管理，锁定在 mise.toml |
| 异步运行时 | tokio | 1.52 | features: full |
| TUI 框架 | ratatui | 0.30 | 最新稳定版，内置 crossterm backend |
| TUI 后端 | crossterm | 0.29 | ratatui 0.30 配套版本 |
| SSH 协议 | russh | 0.60 | 纯 Rust 实现，无 C 依赖 |
| SFTP 协议 | russh-sftp | 2.1 | 配套 russh 的 SFTP subsystem |
| 模糊搜索 | nucleo | 0.5 | helix 编辑器同款，高性能 |
| SSH Config 解析 | ssh2-config | 0.7 | 解析 ~/.ssh/config |
| 配置序列化 | serde + toml | serde 1.0, toml 1.1 | derive 宏 + TOML 读写 |
| 错误处理 | anyhow | 1.0 | MVP 阶段简化错误处理 |
| 跨平台路径 | dirs | 6.0 | XDG 规范目录 |
| 主机 ID | uuid | 1.23 | 预留依赖，当前实现未使用 |
| 终端模拟器 | alacritty_terminal | — | VT100/xterm 状态机，SSH 嵌入式终端渲染（v0.2 引入）|
| 时间处理 | chrono | 0.4 | 连接历史时间戳 RFC3339 序列化（v0.4 引入）|

### 关于 russh + russh-sftp 的风险说明

`russh-sftp` 相比 `ssh2-rs`（基于 libssh2）成熟度较低。已知的潜在问题：
- 符号链接处理可能不完善
- 大文件传输的稳定性未经大规模验证

**降级方案**：如果 v0.1 实装中遇到 SFTP 关键缺陷，切换到 `ssh2-rs`。代价是引入 C 依赖（libssh2），影响交叉编译便利性，但功能稳定性更有保障。架构设计上通过 trait 抽象 SFTP 操作层，使切换成本可控。

## 整体架构

```
┌─────────────────────────────────────────────────────┐
│                     main.rs                         │
│                 bootstrap App::run()                │
├─────────────────────────────────────────────────────┤
│                     app.rs                          │
│                  App State Machine                  │
├─────────────────────────────────────────────────────┤
│           ┌──────────┐                              │
│           │ TUI Layer │ (ratatui + crossterm)       │
│           ├──────────┤                              │
│           │ 主界面    │ 搜索框 + 主机列表 + 快捷键栏   │
│           │ SSH 终端  │ 嵌入式终端 widget + 状态栏     │
│           │ SFTP 面板 │ 单面板浏览器 + 进度条          │
│           └────┬─────┘                              │
│                │ 事件                                │
│           ┌────▼─────┐                              │
│           │ 事件循环  │ stdin 字节流 + Tick           │
│           └────┬─────┘                              │
│                │                                    │
│  ┌─────────────┼─────────────┐                      │
│  │             │             │                      │
│  ▼             ▼             ▼                      │
│ ┌──────┐  ┌──────┐  ┌───────────┐                  │
│ │Config│  │ SSH  │  │   SFTP    │                  │
│ │管理   │  │连接   │  │ 文件操作   │                  │
│ └──┬───┘  └──┬───┘  └─────┬─────┘                  │
│    │         │             │                        │
│    ▼         └──────┬──────┘                        │
│ hosts.toml     ┌────▼────┐                          │
│ ~/.ssh/config  │  russh  │ (SSH + SFTP 共享连接)     │
│                └─────────┘                          │
└─────────────────────────────────────────────────────┘
```

## 模块设计

### 目录结构

```
src/
├── main.rs              # 入口，初始化 tokio + TUI
├── app.rs               # App 状态机，模式管理
├── config/
│   ├── mod.rs           # 配置模块入口
│   ├── host.rs          # Host 数据结构
│   ├── store.rs         # TOML 读写
│   ├── ssh_config.rs    # ~/.ssh/config 解析与导入
│   └── history.rs       # 连接历史（history.toml），时间戳记录与查询
├── ssh/
│   ├── mod.rs           # SSH 模块入口
│   ├── session.rs       # russh 会话管理
│   ├── auth.rs          # 认证策略（agent → key → password）
│   └── terminal.rs      # alacritty_terminal 封装，维护虚拟屏幕状态
├── sftp/
│   ├── mod.rs           # SFTP 模块入口
│   ├── client.rs        # SFTP 操作封装
│   └── transfer.rs      # 文件传输 + 进度回调
├── tui/
│   ├── mod.rs           # TUI 模块入口
│   ├── event.rs         # 事件循环（键盘 + 异步事件合并）
│   ├── views/
│   │   ├── main_view.rs     # 主界面（搜索 + 列表）
│   │   ├── ssh_view.rs      # SSH 终端视图（包含 TerminalView widget）
│   │   ├── sftp_view.rs     # SFTP 文件浏览器
│   │   ├── edit_view.rs     # 主机新建/编辑全屏表单
│   │   ├── import_view.rs   # SSH config 手动导入选择视图
│   │   └── password_dialog.rs  # 密码输入弹窗
│   └── widgets/
│       ├── search_input.rs  # 搜索框组件
│       ├── host_list.rs     # 主机列表组件
│       ├── file_list.rs     # 文件列表组件
│       ├── progress_bar.rs  # 传输进度条
│       ├── status_bar.rs    # 底部快捷键提示栏
│       ├── tag_editor.rs    # chip 标签编辑器（TagEditorState + TagEditor widget）
│       └── confirm_dialog.rs  # 通用确认弹窗
└── utils/
    ├── mod.rs
    └── fuzzy.rs         # 模糊搜索封装
```

### 模块职责

#### `app.rs` — 应用状态机

```rust
enum AppMode {
    Main,             // 主界面：搜索 + 主机列表
    Ssh,              // SSH 嵌入式终端模式
    Sftp,             // SFTP 文件浏览器
    Edit,             // 主机新建/编辑全屏表单
    ImportSshConfig,  // SSH config 手动导入选择视图
}
```

**模式转换规则**：

```
          Enter              Ctrl-\
 Main ──────────→ Ssh ◄──────────────→ Sftp
  ▲                │                     │
  │    exit/Ctrl-D │ 双 Ctrl-C / q       │
  └────────────────┘     ────────────────┘
        s
 Main ──────→ Sftp（直接进入 SFTP，不经过 SSH 模式）
        n/e
 Main ──────→ Edit（新建/编辑主机，Ctrl-S 保存，ESC 取消）
        i
 Main ──────→ ImportSshConfig（选择导入，Enter 确认，ESC 取消）
        首次启动（hosts 为空且未提示过）
 Main ← ConfirmDialog → ImportSshConfig
```

#### `config/host.rs` — 主机数据结构

```rust
struct Host {
    id: String,               // 唯一标识；当前导入主机默认使用 alias
    alias: String,            // 显示名称
    hostname: String,         // IP 或域名
    port: u16,                // 默认 22
    user: String,             // 用户名
    identity_files: Vec<PathBuf>,  // 私钥路径列表
    proxy_jump: Option<String>,    // 跳板机
    tags: Vec<String>,        // 标签
    description: String,      // 描述
    source: HostSource,       // 来源：SshConfig | Manual
}

enum HostSource {
    SshConfig,  // 从 ~/.ssh/config 导入
    Manual,     // 用户手动添加
}
```

#### `config/store.rs` — 配置持久化

配置文件位置：`~/.config/sush/hosts.toml`

```toml
[metadata]
ssh_config_hash = "abc123"  # 用于检测 ssh config 变更

[[hosts]]
id = "prod-web-01"
alias = "prod-web-01"
hostname = "192.168.1.10"
port = 22
user = "deploy"
identity_files = ["~/.ssh/id_ed25519"]
tags = ["web", "prod"]
description = ""
source = "ssh_config"
```

#### `config/ssh_config.rs` — SSH Config 导入

**导入策略**：
1. 读取 `~/.ssh/config`
2. 计算文件内容 hash，与上次导入对比
3. hash 变化时执行增量同步：
   - 新 Host → 添加（source = SshConfig）
   - 已有 Host → 保持原样，不用 ssh config 覆盖
   - ssh config 中删除的 Host → 不自动删除
4. 从每个 `Host` 条目中选择第一个非通配符、非反选 pattern 作为 alias；若不存在则跳过
5. `HostName` 缺失时回退 alias，`Port` 默认 `22`，`ProxyJump` 仅取第一个值

#### `ssh/session.rs` — SSH 会话

```rust
struct ActiveSession {
    handle: russh::client::Handle,
    channel: Option<russh::Channel>,  // 当前 PTY channel
}
```

**关键设计**：SSH 和 SFTP 共享同一条 TCP 连接。`ActiveSession` 持有底层 SSH handle；SSH 接管模式使用 PTY channel，SFTP 操作按需基于同一 handle 打开 subsystem channel。切换模式时不需要重新建立 TCP 连接。

#### `ssh/auth.rs` — 认证流程

```
尝试 ssh-agent
  ├── 成功 → 连接
  └── 失败 ↓
尝试 IdentityFile（按配置顺序）
  ├── 无密码私钥 → 直接使用
  ├── 加密私钥 → 弹出密码输入框后重试
  ├── 成功 → 连接
  └── 全部失败 ↓
弹出密码输入框，尝试密码认证
  ├── 成功 → 连接
  └── 失败 → 调用层输出错误并保持在主界面
```

#### `sftp/transfer.rs` — 文件传输

```rust
struct TransferProgress {
    filename: String,
    total_bytes: u64,
    transferred_bytes: u64,
    state: TransferState,
}

enum TransferState {
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}
```

传输使用固定大小的缓冲区（32KB）逐块读写，每块完成后通过 channel 发送进度更新到 App 层；底部进度条显示方向、文件名和已传输/总大小。

#### `tui/event.rs` — 事件循环

```rust
enum AppEvent {
    Input(Vec<u8>),  // stdin 原始字节流
    Tick,            // 周期性时钟，用于刷新/轮询
}
```

事件循环由 `EventBus` 提供 `Input(Vec<u8>)` 和 `Tick` 两类事件：
- `Input`：后台阻塞读取 stdin 后投递，由 App 在不同模式下分别解析为 TUI 按键或 SSH 原始输入
- `Tick`：固定间隔轮询，用于处理终端 resize、传输状态收尾和其他周期性逻辑
- SSH channel 消息不经过 `AppEvent`，而是在 SSH 模式下由 `tokio::select!` 与 `EventBus` 并行等待

## SSH 嵌入式终端模式（v0.2）

v0.2 起，SSH 模式从接管模式切换为**嵌入式终端模拟器模式**：TUI 始终处于 alternate screen，远程 PTY 输出由 `alacritty_terminal` 维护虚拟屏幕状态，再由 `TerminalView` widget 渲染到 ratatui 画布。

**架构变化**：

- 不再执行 `LeaveAlternateScreen` / `EnterAlternateScreen`，TUI 全程运行
- SSH 输出字节流喂给 `alacritty_terminal::Term`，由其维护 cell 网格（字符、颜色、属性）
- `TerminalView` widget 将 `Term::grid()` 映射到 ratatui `Cell`，随每帧渲染
- 状态栏在 SSH 模式下持续可见，显示主机名与快捷键提示
- `Ctrl-\`（字节 `0x1c`）仍作为前缀键，在 TUI 事件层处理，不透传给远程

**终端 resize**：ratatui 分配给 `TerminalView` 的区域发生变化时，同步调用 `Term::resize()` 与 `session.resize_pty()`。

**颜色映射**：`alacritty_terminal` 的 `Color::Named` → ratatui named colors，`Color::Indexed(n)` → ratatui `Color::Indexed(n)`，`Color::Rgb(r,g,b)` → ratatui `Color::Rgb(r,g,b)`。

## 数据流

### SSH 模式

```
键盘 stdin (raw bytes)
  │
  ▼
EventBus → handle_ssh_input()
  ├── Ctrl-\ 前缀键 ──→ 切换到 SFTP（TUI 内部处理）
  └── 其他字节 ──→ russh channel.data() ──→ 远程 PTY
                                               │
                                               ▼ (远程 PTY 输出)
                                    russh ChannelMsg::Data
                                               │
                                               ▼
                               TerminalEmulator.process(bytes)
                               （alacritty_terminal::Term 更新 cell 网格）
                                               │
                                               ▼
                                     ratatui 渲染帧
                               └── TerminalView widget
                                     读取 Term::grid() → 渲染 cell
```

### SFTP 模式

```
键盘事件
  │
  ▼
TUI 事件循环 ──→ ratatui 渲染
  │
  │ (用户操作)
  ▼
SFTP 操作 ──→ russh-sftp ──→ 远程文件系统
  │
  ▼ (进度回调)
进度条更新 ──→ ratatui 渲染
```

## 配置与数据目录

```
~/.config/sush/
├── hosts.toml        # 主机配置（含导入的和手动添加的）
└── settings.toml     # 全局设置（预留，v0.1 可能不需要）
```

当前实现配置目录为：
- `dirs::home_dir()/.config/sush/`

当前代码尚未使用 `dirs::config_dir()` 做平台差异化处理。

## 构建与分发

### 编译目标

| 平台 | Target Triple |
|------|--------------|
| macOS ARM | aarch64-apple-darwin |
| macOS Intel | x86_64-apple-darwin |
| Linux x86_64 | x86_64-unknown-linux-musl (静态链接) |
| Windows x86_64 | x86_64-pc-windows-msvc |

### CI/CD

当前仓库尚未提交 CI workflow；后续可补充：
- push 到主分支时运行测试 + clippy
- tag `v*` 时自动构建各平台并发布 Release

Linux 使用 musl 静态链接，确保单文件无动态库依赖。

## 错误处理策略

v0.1 采用简单策略：
- 连接/SFTP/导航/传输启动失败 → 通过 `eprintln!` 输出错误，保持或返回当前可交互界面
- 认证过程中的密码输入 → 使用 TUI 密码弹窗
- 传输完成、失败或取消 → 进度条短暂保留后恢复快捷键栏
- 配置文件损坏 → 提示错误，使用默认空配置启动
- 不做自动重连（v0.2+ 考虑）
