# sush

> SSH 和 SFTP，终于住进同一屋檐下。

`sush` 是一个小巧、快速、纯终端的工具，用于管理 SSH 连接和 SFTP 文件传输——全程不离开键盘。

---

## 问题是什么

你 SSH 进了一台服务器。然后发现需要拿个文件。于是你：

1. 打开新的终端标签页
2. 手忙脚乱地敲 `sftp user@host`
3. 忘记了刚才在哪个路径
4. 放弃，改用 `scp`，凭记忆输路径
5. 路径还是输错了

`sush` 的解法是：把 SSH 和 SFTP 当成同一个 session 的两个视图。按 `Ctrl-\` 随时切换，就这么简单。

---

## 演示

```
┌─ sush ─────────────────────────────────────────────┐
│                                                     │
│  > prod█                                            │
│                                                     │
│  ┌───────────────────────────────────────────────┐  │
│  │ ● prod-web-01   192.168.1.10          web    │  │
│  │   prod-db-01    192.168.1.20           db    │  │
│  │   prod-cache    192.168.1.30        cache    │  │
│  └───────────────────────────────────────────────┘  │
│                                                     │
│  /:搜索  Enter:SSH  s:SFTP  q:退出                  │
└─────────────────────────────────────────────────────┘
```

直接输入即可模糊搜索。按 Enter 连接。按 `Ctrl-\` 随时切到 SFTP 文件浏览器（SSH 会话保持不动）。再按一次 `Ctrl-\` 跳回去。

---

## 功能

**零摩擦 SSH**
- 启动时自动读取 `~/.ssh/config`，主机已经在列表里了
- 多维模糊搜索：主机名、IP、用户名、标签、描述全部匹配
- 嵌入式终端模拟器：`vim`、`tmux`、`htop` 全都正常运行

**无缝 SSH ↔ SFTP 切换**
- `Ctrl-\` 在 SSH shell 和 SFTP 文件浏览器之间翻转
- SSH 和 SFTP 共享同一条 TCP 连接，无需重新认证
- 切换瞬间完成，上下文保留

**真正好用的 SFTP**
- `Tab` 在本地/远程视图间切换
- `d` 下载，`u` 上传，底部实时进度条
- `Enter` 进入目录，`..` 返回上级

**快**
- 启动时间 < 200ms
- 搜索响应 < 50ms
- 空闲内存 < 30MB

---

## 安装

### 下载二进制（推荐）

从 [GitHub Releases](https://github.com/lichen/sush.sh/releases) 下载对应平台的文件：

| 平台 | 文件 |
|------|------|
| macOS（Apple Silicon）| `sush-macos-arm64` |
| macOS（Intel）| `sush-macos-x86_64` |
| Linux x86_64 | `sush-linux-x86_64` |
| Windows x86_64 | `sush-windows-x86_64.exe` |

```sh
# macOS / Linux
chmod +x sush-*
mv sush-* /usr/local/bin/sush
sush
```

### 从源码构建

```sh
git clone https://github.com/lichen/sush.sh
cd sush.sh
cargo build --release
./target/release/sush
```

需要 Rust 1.95+，无其他依赖。

---

## 快速上手

```sh
sush
```

首次启动时，`sush` 会自动导入 `~/.ssh/config` 中的主机。如果没有配置文件，从空列表开始（TUI 主机编辑器将在 v0.3 上线）。

**主界面导航**

| 按键 | 动作 |
|------|------|
| `/` 或直接输入 | 聚焦搜索框 |
| `↑` / `↓` | 在主机列表中移动 |
| `Enter` | SSH 连接 |
| `s` | 打开 SFTP 浏览器 |
| `q` | 退出 |

**SSH 模式**

| 按键 | 动作 |
|------|------|
| `Ctrl-\` | 切换到 SFTP 文件浏览器 |
| `exit` / `Ctrl-D` | 断开连接，返回主界面 |

**SFTP 模式**

| 按键 | 动作 |
|------|------|
| `Tab` | 切换本地 / 远程视图 |
| `Enter` | 进入目录 |
| `d` | 下载选中文件 |
| `u` | 上传选中文件 |
| `Ctrl-\` | 切回 SSH shell |
| `Ctrl-C` × 2 | 返回主界面 |

---

## 认证方式

`sush` 按以下顺序尝试认证：

1. **ssh-agent** — 若 `SSH_AUTH_SOCK` 已设置，优先使用
2. **IdentityFile** — 读取 `~/.ssh/config` 中的密钥路径；有密码保护时会在 TUI 弹出输入框
3. **密码认证** — 所有方法失败后，在 TUI 中弹出密码输入框

---

## 工作原理

`sush` 使用**嵌入式终端模拟器**（基于 [alacritty_terminal](https://github.com/alacritty/alacritty)）。连接主机后，远程 PTY 的输出字节被送入进程内的 VT100/xterm 状态机，渲染结果以 ratatui widget 的形式显示——整个 SSH 会话期间，sush 的界面（状态栏、快捷键提示）始终可见。

- 终端程序通过完整的 VT100 模拟正常工作
- `Ctrl-\` 在 TUI 层被截取为前缀键；其他所有按键直接转发给远程
- SSH 和 SFTP 通过独立的 channel 共享同一条 TCP 连接，模式切换即时完成，无需重新认证

---

## 版本规划

| 版本 | 重点 |
|------|------|
| **v0.1** ✅ | SSH 连接 · SFTP 浏览 · 文件上传下载 · `Ctrl-\` 切换 |
| **v0.2** ✅ | 嵌入式终端模拟器 · SSH 会话期间 TUI 界面始终可见 |
| v0.3 | TUI 主机编辑 · 标签管理 · 描述字段 |
| v0.4 | 连接历史 · 最近使用排序 · 连通性检测 |
| v0.5 | path 类型标签 · 虚拟文件夹导航 |
| v0.6 | 凭证加密存储（master password） |
| v0.7 | 文件夹递归传输 · 远程文件编辑 · 双面板 SFTP |
| v0.8 | 端口转发管理 · ProxyJump 多级跳板 · SOCKS5 代理 |
| v1.0 | Homebrew/AUR/Scoop 分发 · man page · 全平台测试 |

---

## 技术栈

- [russh](https://github.com/Eugeny/russh) — 纯 Rust SSH 实现，无 C 依赖
- [alacritty_terminal](https://github.com/alacritty/alacritty) — VT100/xterm 终端模拟器
- [ratatui](https://ratatui.rs) — TUI 渲染框架
- [nucleo](https://github.com/helix-editor/nucleo) — 模糊搜索（Helix 编辑器同款）
- [tokio](https://tokio.rs) — 异步运行时

单文件二进制，无系统依赖，无 libssh2，无 OpenSSL。

---

## 许可证

MIT
