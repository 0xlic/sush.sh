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
| 主机 ID | uuid | 1.23 | v4 随机 UUID |

### 关于 russh + russh-sftp 的风险说明

`russh-sftp` 相比 `ssh2-rs`（基于 libssh2）成熟度较低。已知的潜在问题：
- 符号链接处理可能不完善
- 大文件传输的稳定性未经大规模验证

**降级方案**：如果 v0.1 实装中遇到 SFTP 关键缺陷，切换到 `ssh2-rs`。代价是引入 C 依赖（libssh2），影响交叉编译便利性，但功能稳定性更有保障。架构设计上通过 trait 抽象 SFTP 操作层，使切换成本可控。

## 整体架构

```
┌─────────────────────────────────────────────────────┐
│                     main.rs                         │
│                   App State Machine                 │
├─────────────────────────────────────────────────────┤
│           ┌──────────┐                              │
│           │ TUI Layer │ (ratatui + crossterm)       │
│           ├──────────┤                              │
│           │ 主界面    │ 搜索框 + 主机列表 + 快捷键栏   │
│           │ SFTP 面板 │ 单面板浏览器 + 进度条          │
│           └────┬─────┘                              │
│                │ 事件                                │
│           ┌────▼─────┐                              │
│           │ 事件循环  │ 键盘/终端/SSH/传输事件         │
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
│   └── ssh_config.rs    # ~/.ssh/config 解析与导入
├── ssh/
│   ├── mod.rs           # SSH 模块入口
│   ├── session.rs       # russh 会话管理
│   └── auth.rs          # 认证策略（agent → key → password）
├── sftp/
│   ├── mod.rs           # SFTP 模块入口
│   ├── client.rs        # SFTP 操作封装
│   └── transfer.rs      # 文件传输 + 进度回调
├── tui/
│   ├── mod.rs           # TUI 模块入口
│   ├── event.rs         # 事件循环（键盘 + 异步事件合并）
│   ├── views/
│   │   ├── main_view.rs     # 主界面（搜索 + 列表）
│   │   ├── sftp_view.rs     # SFTP 文件浏览器
│   │   └── password_dialog.rs  # 密码输入弹窗
│   └── widgets/
│       ├── search_input.rs  # 搜索框组件
│       ├── host_list.rs     # 主机列表组件
│       ├── file_list.rs     # 文件列表组件
│       ├── progress_bar.rs  # 传输进度条
│       └── status_bar.rs    # 底部快捷键提示栏
└── utils/
    ├── mod.rs
    └── fuzzy.rs         # 模糊搜索封装
```

### 模块职责

#### `app.rs` — 应用状态机

```rust
enum AppMode {
    Main,           // 主界面：搜索 + 主机列表
    Ssh(SshState),  // SSH 接管模式
    Sftp(SftpState),// SFTP 文件浏览器
}

struct App {
    mode: AppMode,
    hosts: Vec<Host>,
    search_query: String,
    filtered_hosts: Vec<usize>,  // 搜索结果索引
    selected_index: usize,
    session: Option<ActiveSession>,  // 当前活跃的 SSH/SFTP 连接
}
```

**模式转换规则**：

```
           Enter              Ctrl-Space
  Main ──────────→ Ssh ◄──────────────→ Sftp
   ▲                │                     │
   │    exit/Ctrl-D │     双 Ctrl-C       │
   └────────────────┘     ────────────────┘
         F2
  Main ──────→ Sftp（直接进入 SFTP，不经过 SSH）
```

#### `config/host.rs` — 主机数据结构

```rust
struct Host {
    id: String,               // 唯一标识（UUID 或 alias）
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
   - 已有 Host 配置变化 → 更新
   - ssh config 中删除的 Host → 不自动删除（避免误删用户数据）
4. 跳过通配符 Host（`Host *`、`Host 192.168.*`）

#### `ssh/session.rs` — SSH 会话

```rust
struct ActiveSession {
    ssh_handle: russh::client::Handle,
    channel: Option<russh::Channel>,  // PTY channel
    sftp: Option<SftpClient>,         // 复用同一 SSH 连接的 SFTP subsystem
}
```

**关键设计**：SSH 和 SFTP 共享同一条 TCP 连接。SSH 通过 PTY channel，SFTP 通过 SFTP subsystem channel，两者互不干扰。切换模式时不需要重新连接。

#### `ssh/auth.rs` — 认证流程

```
尝试 ssh-agent
  ├── 成功 → 连接
  └── 失败 ↓
尝试 IdentityFile（按配置顺序）
  ├── 无密码私钥 → 直接使用
  ├── 加密私钥 → 弹出密码输入框解密
  ├── 成功 → 连接
  └── 全部失败 ↓
弹出密码输入框，尝试密码认证
  ├── 成功 → 连接
  └── 失败 → 显示错误，返回主界面
```

#### `sftp/transfer.rs` — 文件传输

```rust
struct TransferProgress {
    filename: String,
    total_bytes: u64,
    transferred_bytes: u64,
    speed_bps: u64,  // bytes per second
    state: TransferState,
}

enum TransferState {
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}
```

传输使用固定大小的缓冲区（32KB）逐块读写，每块完成后通过 channel 发送进度更新到 TUI 层。

#### `tui/event.rs` — 事件循环

```rust
enum AppEvent {
    Key(KeyEvent),              // 键盘输入
    Resize(u16, u16),           // 终端 resize
    SshOutput(Vec<u8>),         // 远程 SSH 输出
    TransferProgress(TransferProgress),  // 传输进度更新
    ConnectionLost(String),     // 连接断开
}
```

事件循环使用 `tokio::select!` 合并多个异步事件源：
- crossterm 的键盘/终端事件
- russh 的 SSH 数据通道
- 传输进度通道

## SSH 接管模式 — 前缀键实现细节

进入 SSH 模式时：

1. 终端切换到 raw mode（crossterm 已处理）
2. 禁用 IXON 流控（`stty -ixon`，确保 Ctrl-Space 等键不被终端拦截）
3. sush 读取 stdin 字节流：
   - 检测到 `Ctrl-Space`（字节 `0x00`）→ 触发模式切换
   - 其他所有字节 → 原样转发给远程 PTY
4. 远程 PTY 输出 → 原样写入 stdout

**退出 SSH 模式时**：恢复终端设置。

**Ctrl-Space 的字节表示**：在 raw mode 下，Ctrl-Space 产生 `NUL`（`0x00`）。这在绝大多数远程程序中不会被使用，冲突概率极低。

## 数据流

### SSH 模式

```
键盘 stdin
  │
  ▼
sush I/O 转发层 ──(检测前缀键)──→ 切换到 SFTP
  │
  │ (透传)
  ▼
russh channel.data() ──→ 远程 PTY
  │
  ▼ (远程输出)
russh channel.on_data() ──→ stdout
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

遵循 XDG 规范：
- Linux/macOS: `~/.config/sush/`
- Windows: `%APPDATA%\sush\`

使用 `dirs` crate 获取跨平台路径。

## 构建与分发

### 编译目标

| 平台 | Target Triple |
|------|--------------|
| macOS ARM | aarch64-apple-darwin |
| macOS Intel | x86_64-apple-darwin |
| Linux x86_64 | x86_64-unknown-linux-musl (静态链接) |
| Windows x86_64 | x86_64-pc-windows-msvc |

### CI/CD

GitHub Actions workflow：
- 每次 push 到 main：运行测试 + clippy
- tag `v*`：自动构建所有平台 + 创建 GitHub Release + 上传二进制

Linux 使用 musl 静态链接，确保单文件无动态库依赖。

## 错误处理策略

v0.1 采用简单策略：
- 连接失败 → 在 TUI 中显示错误弹窗，返回主界面
- 传输失败 → 在进度栏显示错误信息
- 配置文件损坏 → 提示错误，使用默认空配置启动
- 不做自动重连（v0.2+ 考虑）
