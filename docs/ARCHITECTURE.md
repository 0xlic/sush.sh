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
| 文件监听 | notify | 8.2 | 远程编辑时监听本地临时文件变更（v0.7 引入） |
| 临时工作区 | tempfile | 3.10 | 远程编辑临时文件与工作目录（v0.7 引入） |
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
│           │ 主界面    │ 搜索框 + 目录栏 + 主机列表 + 快捷键栏 │
│           │ SSH 终端  │ 嵌入式终端 widget + 状态栏     │
│           │ SFTP 面板 │ 自适应双面板浏览器 + 紧凑传输 badge │
│           │ 转发管理  │ 双面板规则视图 + daemon 状态     │
│           └────┬─────┘                              │
│                │ 事件                                │
│           ┌────▼─────┐                              │
│           │ 事件循环  │ stdin 字节流 + Tick           │
│           └────┬─────┘                              │
│                │                                    │
│  ┌─────────────┼─────────────┐                      │
│  │             │             │                      │
│  ▼             ▼             ▼                      │
│ ┌──────┐  ┌──────┐  ┌───────────┐  ┌───────────┐   │
│ │Config│  │ SSH  │  │   SFTP    │  │  tunnel   │   │
│ │管理   │  │连接   │  │ 文件操作   │  │ daemon/IPC │   │
│ └──┬───┘  └──┬───┘  └─────┬─────┘  └─────┬─────┘   │
│    │         │             │              │         │
│    ▼         └──────┬──────┴──────────────┘         │
│ hosts.toml     ┌────▼────┐                          │
│ ~/.ssh/config  │  russh  │ (SSH / SFTP / Port Forward 共享能力) │
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
│   ├── proxy_jump.rs    # ProxyJump 单跳 direct-tcpip + connect_stream
│   └── terminal.rs      # alacritty_terminal 封装，维护虚拟屏幕状态
├── tunnel/
│   ├── mod.rs           # 端口转发模块入口
│   ├── ipc.rs           # daemon <-> TUI 的 Unix socket 协议
│   ├── daemon.rs        # 守护进程、规则状态机、自动重连
│   ├── client.rs        # TUI 侧 IPC client，负责自动拉起 daemon
│   └── forward.rs       # 本地 / 远程 / 动态转发实现
├── sftp/
│   ├── mod.rs           # SFTP 模块入口
│   ├── client.rs        # SFTP 操作封装
│   └── transfer.rs      # 文件传输 + 进度回调
├── tui/
│   ├── mod.rs           # TUI 模块入口
│   ├── event.rs         # 事件循环（键盘 + 异步事件合并）
│   ├── views/
│   │   ├── main_view.rs     # 主界面（搜索 + 目录栏 + 主机列表）
│   │   ├── folder_view.rs   # 目录树状态与跳转逻辑（供主界面复用）
│   │   ├── ssh_view.rs      # SSH 终端视图（包含 TerminalView widget）
│   │   ├── sftp_view.rs     # SFTP 文件浏览器
│   │   ├── edit_view.rs     # 主机新建/编辑全屏表单
│   │   ├── import_view.rs   # SSH config 手动导入选择视图
│   │   ├── forwarding_view.rs  # 转发规则双面板状态视图
│   │   ├── forward_edit.rs     # 转发规则新建/编辑弹层
│   │   └── password_dialog.rs  # 密码输入弹窗
│   └── widgets/
│       ├── search_input.rs  # 搜索框组件
│       ├── host_list.rs     # 主机列表组件
│       ├── file_list.rs     # 文件列表组件
│       ├── progress_bar.rs  # 旧传输进度条组件（当前未启用）
│       ├── status_bar.rs    # 底部快捷键提示栏
│       ├── tag_editor.rs    # chip 标签编辑器（TagEditorState + TagEditor widget）
│       └── confirm_dialog.rs  # 通用确认弹窗
└── utils/
    ├── mod.rs
    ├── fuzzy.rs         # 模糊搜索封装
    └── open.rs          # 按平台生成并启动默认应用打开命令（v0.7 引入）
```

### 模块职责

#### `app.rs` — 应用状态机

```rust
enum AppMode {
    Main,             // 主界面：搜索 + 主机列表
    Ssh,              // SSH 嵌入式终端模式
    Sftp,             // SFTP 文件浏览器
    ForwardingManager,// 端口转发管理
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
       p
Main ──────→ ForwardingManager（规则状态、启动/停止、编辑）
```

## v0.5 虚拟目录导航

v0.5 在主界面中集成虚拟目录导航，而不是引入新的主交互页面：

- `f` 切换左侧目录栏显示/隐藏
- 目录栏显示时，主界面布局变为“搜索框 + 左侧目录栏 + 右侧主机列表”
- `FolderViewState` 继续承载目录树、目录跳转与路径筛选逻辑，但作为主界面状态复用
- `j` 目录跳转浮层仅在目录栏焦点下启用
- 搜索框在目录栏显示时展示 `path:/当前目录` 前缀，并先按目录范围过滤，再执行现有模糊搜索

**数据来源**：

- 主机的 `/` 前缀标签用于构建虚拟目录树
- 无路径标签的主机归入根目录 `/`
- 一台主机可因多个路径标签同时出现在多个目录下

**主界面焦点行为**：

- `MainFocus::Directory`：左侧目录栏焦点，右侧主机列表仅作预览
- `MainFocus::HostList`：右侧主机列表焦点，恢复选中高亮、描述行与 TCP probe 状态
- `MainFocus::Search`：搜索框焦点；目录栏显示时执行目录范围内搜索，隐藏时执行全局搜索

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

v0.6 起，登录密码和私钥口令都优先从系统安全存储读取：

- macOS 使用 Keychain
- Windows 使用 Credential Manager
- Linux 使用 Secret Service

凭证不会写入 `hosts.toml`。只有认证成功后，`App` 才会静默尝试把本次手工输入的登录密码或私钥口令保存到系统钥匙串；保存失败不会影响当前连接，只会把失败原因写入本地 metadata，供下次再次输入时提示。

## v0.8 端口转发守护进程

v0.8 新增 `tunnel/` 模块，把“转发规则配置”和“转发运行时”解耦：

- `Host.forwards` 持久化规则定义，规则类型包括 `Local`、`Remote`、`Dynamic`
- `daemon.rs` 在 Unix 平台启动独立守护进程，监听 `~/.config/sush/daemon.sock`
- `client.rs` 由 TUI 侧通过 IPC 拉起或连接 daemon，查询状态并发送 `start/stop/status`
- `forward.rs` 负责三种转发的运行时：
  - 本地转发：本地监听 + `direct-tcpip`
  - 动态转发：最小 SOCKS5 CONNECT + `direct-tcpip`
  - 远程转发：`tcpip_forward` + `forwarded-tcpip` 回连到本地端口

### 状态模型

每条规则在 daemon 内维护独立状态：

- `Stopped`：未运行
- `Connecting`：正在建立 SSH / ProxyJump / 监听端口
- `Running`：转发正常运行
- `Reconnecting`：连接断开后按退避重试
- `Error`：超过最大重试次数，或配置错误（如跳板机不存在）

### ProxyJump

v0.8 只支持单跳 ProxyJump：

1. 使用跳板机 `Host` 配置建立并认证第一段 SSH
2. 在跳板机上打开到目标主机的 `direct-tcpip` channel
3. 把该 channel 转成 stream，再通过 `russh::client::connect_stream` 建立第二段 SSH
4. 再对目标主机执行认证与后续转发

跳板机引用使用 `Host.proxy_jump = Some(alias)`；daemon 启动规则时按 alias 在当前配置中解析对应主机。

Linux 若没有可用的 Secret Service：

- 禁止保存凭证
- 允许本次临时输入继续连接
- 下次输入时提示安装 `gnome-keyring` 或 `kwallet`

#### `sftp/transfer.rs` — 文件传输

```rust
struct TransferProgress {
    filename: String,
    total_bytes: u64,
    transferred_bytes: u64,
    state: TransferState,
    current_file_index: usize,
    total_files: usize,
}

enum TransferState {
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}
```

传输使用固定大小的缓冲区（32KB）逐块读写，每块完成后通过 channel 发送进度更新到 App 层；单文件传输使用 `1/1` 进度，递归传输在 App 层复用同一条链路并覆写为 `N/M` 聚合进度。

## 传输队列与后台传输（v0.7）

v0.7 在现有传输链路上增加当前连接范围内的单一 FIFO 队列，用最小改动支持后台顺序继续：

- `App` 维护 `active_transfer`、`active_recursive_transfer` 与 `queued_transfers`
- 上传、下载、递归任务在入队前就展开成明确 `QueuedTransfer`
- 同一时间只运行一个活动任务；当前任务结束后自动启动下一个
- 离开 `SFTP` 只切换视图，不影响后台队列；真正断开当前连接时统一清空队列与运行态
- `main_view.rs`、`ssh_view.rs`、`sftp_view.rs` 共享 `TransferBadge`，底部右侧显示 `↑ 2/10 37%` 之类的紧凑状态

当前限制：

- 仍然只支持单 worker 串行传输，不做并行
- 队列不跨连接、不跨进程持久化
- badge 仅显示方向、队列位置与当前文件百分比，不显示 ETA、速率或文件名

## 断点续传（v0.7）

v0.7 的断点续传采用最小实现，仍然只落在单文件传输层：

- 下载时读取本地目标文件当前大小；若大小小于等于远端源文件总大小，则本地文件追加写入，远端句柄 seek 到相同偏移继续读取
- 上传时读取远端目标文件当前大小；若大小小于等于本地源文件总大小，则本地文件 seek 到相同偏移，远端文件以 append 模式继续写入
- 若目标侧大小大于源文件大小，则判定为不可安全续传，回退为从 0 重传该文件
- 递归传输和后台队列不新增专门状态，而是继续逐文件复用同一套 `upload()` / `download()` helper

当前限制：

- 只按文件大小决定是否续传，不做内容哈希校验
- 不保存跨进程、跨重启的续传元数据
- 目录本身没有独立“续传快照”，只是其中每个普通文件单独判断

## 递归目录传输（v0.7）

v0.7 在现有 SFTP 自适应面板上增加目录递归上传/下载能力，仍然复用当前前台单任务模型：

- `sftp/transfer.rs` 负责构建本地或远端目录计划
- `App` 持有 `ActiveRecursiveTransfer` 作为递归任务运行态
- 目录准备完成后，`App` 逐文件复用现有单文件 `upload()` / `download()` 逻辑
- UI 不再使用整行进度条，而是在共享状态栏右侧显示紧凑 badge；递归任务仍然沿用 `N/M` 聚合语义

当前实现语义：

- 保留所选目录本身
- 先创建目录，再顺序传输文件
- 同一时刻只运行一个前台传输任务
- 单文件传输与递归传输共用同一套 `ActiveTransfer` 进度轮询链路

## SFTP 多选批量操作（v0.7）

v0.7 在自适应双面板 SFTP 之上增加每侧独立的多选状态与批量操作入口：

- `SftpPaneState` 维护本地/远程各自的 `selection`、`anchor` 和最近一次 `Space` 时间戳
- `App::handle_sftp_key()` 只对当前激活面板处理 `Space`、双击 `Space`、`Esc`
- 视图层在 `sftp_view.rs` 中根据当前激活面板是否处于多选态切换底部提示栏
- `file_list.rs` 负责为选中项渲染最小可视标记

当前实现语义：

- 单击 `Space` 切换当前项选中状态并更新锚点
- 在另一行上快速双击 `Space`，选中锚点到当前项之间的闭区间
- 本地多选下按 `u` 生成顺序批量上传计划；远程多选下按 `d` 生成顺序批量下载计划
- `D` 进入 SFTP 删除确认流；确认后按当前激活面板逐项删除，并在成功后触发对应面板刷新
- 批量传输和批量删除完成后，当前激活面板的多选状态被清空

## 远程文件编辑（v0.7）

v0.7 第一阶段在现有 SFTP 自适应面板上增加“远程文件编辑桥接”能力，而不是内置文本编辑器：

- 用户在 SFTP 远程视图中选中文件后按 `e`
- `App` 下载远端文件到本地临时工作区
- `utils/open.rs` 按平台调用系统默认图形化应用打开本地副本
- 本地文件保存后，`App` 通过文件事件与轮询混合检测内容变化
- 变化被确认后，自动上传回远端并刷新远程列表

#### `utils/open.rs` — 默认应用启动

`OpenCommand` 负责把“打开本地路径”的需求映射为平台命令：

- macOS：`open <path>`
- Linux：`xdg-open <path>`
- Windows：`cmd /C start "" <path>`

`open_path()` 只负责启动默认应用，不关心业务状态机。

#### `app.rs` — 远程编辑会话状态

```rust
enum RemoteEditSyncState {
    Opening,
    Watching,
    Uploading,
    UploadFailed,
    Closed,
}
```

`RemoteEditSession` 保存：

- `remote_path`
- `local_path`
- `workspace`（临时目录）
- 最近一次成功上传的内容指纹
- 最近一次观测到的内容指纹
- watcher 句柄与事件通道
- 当前同步状态与最近错误

`App` 继续停留在 `AppMode::Sftp`，不新增专门的编辑模式页；远程编辑状态通过底部状态文案反馈。

#### 变更检测策略

远程编辑使用混合策略确认“保存”事件：

- 优先使用 `notify` 监听临时工作区文件变化
- 每个 `Tick` 轮询当前文件内容指纹
- 只有相对于“最近一次成功上传版本”真正发生内容变化时，才触发上传

这样可以覆盖常见 GUI 编辑器的直接写入和原子替换保存行为，同时避免同一内容重复上传。

#### 远端写回策略

为避免自动上传直接把远端目标文件写成半截内容，写回流程采用兼容旧服务端的三步替换：

1. 先把本地临时文件上传到远端同目录临时文件
2. 若目标文件已存在，先把旧文件改名到备份路径
3. 再把新临时文件改名到目标路径；若失败，则尽力把旧文件改回去

如果上传失败：

- 编辑会话继续保留
- 状态栏显示失败原因
- 用户下次再次保存时自动重试

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

### 远程编辑模式（v0.7）

```
SFTP 远程视图选中文件 + e
  │
  ▼
App.start_remote_edit()
  ├── 下载到本地临时文件
  ├── open_path() 调默认图形化应用
  └── 建立文件事件 watcher
            │
            ▼
        AppEvent::Tick
            │
            ▼
     fingerprint_file(local)
            │
   内容变化且不同于最近成功版本？
        ├── 否 → 保持 Watching
        └── 是 → 上传到远端临时文件
                      │
                      ▼
              rename 覆盖目标文件
                      │
          成功 → 刷新远端目录 + 状态栏提示
          失败 → 保持会话，等待下次保存重试
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
- 传输完成、失败或取消 → badge 根据队列状态切换到下一任务或消失
- 配置文件损坏 → 提示错误，使用默认空配置启动
- 不做自动重连（v0.2+ 考虑）
