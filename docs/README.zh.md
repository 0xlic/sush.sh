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
- path 类型标签可在主界面构建虚拟目录侧栏
- 嵌入式终端模拟器：`vim`、`tmux`、`htop` 全都正常运行

**无缝 SSH ↔ SFTP 切换**
- `Ctrl-\` 在 SSH shell 和 SFTP 文件浏览器之间翻转
- SSH 和 SFTP 共享同一条 TCP 连接，无需重新认证
- 切换瞬间完成，上下文保留

**真正好用的 SFTP**
- 宽终端自动左右同屏显示本地与远程面板；窄终端只显示当前激活面板
- `Tab` 在本地/远程面板间切换焦点，并保留各自的选中项
- `d` 下载，`u` 上传，底部右侧显示全局紧凑传输状态
- 文件夹传输会保留所选目录本身，并显示聚合 `N/M` 进度
- `e` 用系统默认图形化应用打开远程文件，并在保存后自动回传
- `Enter` 进入目录，`..` 返回上级
- 当前连接内只有一个 FIFO 传输队列；切回主页、SSH、SFTP 时任务仍会顺序继续

**端口转发管理**
- 主界面按 `p` 进入转发管理器，按主机分组查看和管理转发规则
- 支持本地转发、远程转发和动态转发（SOCKS5）
- 支持单跳 ProxyJump，经跳板机建立转发
- daemon 会显示规则状态，并在可重试错误时自动退避重连

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
| macOS（Apple Silicon）| `sush-aarch64-apple-darwin.tar.xz` |
| macOS（Intel）| `sush-x86_64-apple-darwin.tar.xz` |
| Linux arm64 | `sush-aarch64-unknown-linux-gnu.tar.xz` |
| Linux x86_64 | `sush-x86_64-unknown-linux-gnu.tar.xz` |
| Windows x86 | `sush-i686-pc-windows-msvc.zip` |
| Windows x86_64 | `sush-x86_64-pc-windows-msvc.zip` |

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

需要 Rust 1.95+。凭证持久化依赖操作系统原生安全存储；其中 Linux 需要可用的 Secret Service（如 `gnome-keyring` 或 `kwallet`）。

---

## 快速上手

```sh
sush
```

首次启动时，`sush` 会询问是否从 `~/.ssh/config` 导入主机。也可以按 `n` 手动新建，或按 `i` 随时导入。

**主界面导航**

| 按键 | 动作 |
|------|------|
| `/` 或直接输入 | 聚焦搜索框 |
| `↑` / `↓` | 在主机列表中移动 |
| `Enter` | SSH 连接 |
| `s` | 打开 SFTP 浏览器 |
| `n` | 新建主机 |
| `e` | 编辑选中主机 |
| `d` | 删除选中主机 |
| `i` | 从 `~/.ssh/config` 导入 |
| `f` | 显示/隐藏目录栏 |
| `p` | 打开端口转发管理器 |
| `j` | 跳转目录（目录栏聚焦时） |
| `q` | 退出 |

当目录栏显示时，搜索会自动限定在当前目录范围内，搜索框会显示只读前缀 `path:/当前目录`。

**SSH 模式**

| 按键 | 动作 |
|------|------|
| `Ctrl-\` | 切换到 SFTP 文件浏览器 |
| `exit` / `Ctrl-D` | 断开连接，返回主界面 |

**SFTP 模式**

| 按键 | 动作 |
|------|------|
| `Tab` | 切换本地 / 远程面板焦点 |
| `Space` | 切换当前焦点项的选中状态 |
| `Space` × 2 | 以锚点为起点，选中到当前焦点项之间的闭区间 |
| `Esc` | 取消当前激活面板的多选 |
| `Enter` | 进入目录 |
| `d` | 下载当前远程项；多选时批量下载当前远程选中集 |
| `u` | 上传当前本地项；多选时批量上传当前本地选中集 |
| `D` | 删除当前激活面板里的全部选中项 |
| `e` | 在本地编辑选中的远程文件 |
| `Ctrl-\` | 切回 SSH shell |
| `Ctrl-C` × 2 | 返回主界面 |

在远程视图中按 `e` 后，`sush` 会先把远程文件下载到本地临时工作区，再用操作系统默认应用打开。此后每次保存，sush 都会检测内容变化并自动上传；写回时先写入远端临时文件，必要时先把旧文件移到备份路径，再把新文件切换到目标路径。

当传输的是文件夹时，`sush` 会在目标侧保留所选目录本身，先准备目录结构，再逐文件顺序传输；底部右侧 badge 会显示 `当前任务/总任务` 与当前文件百分比。

SFTP 多选模式下，本地和远程面板分别维护各自的选中集合。按一次 `Space` 会切换当前项选中状态并更新锚点；在另一行上快速双击 `Space` 会选中锚点到当前行之间的闭区间。进入多选后，底部提示栏会切换为批量操作：本地面板显示 `u / D / Esc`，远程面板显示 `d / D / Esc`。

传输任务现在走当前 SSH 连接范围内的单一 FIFO 队列。主页、SSH、SFTP 三个界面的底部右侧都会显示紧凑 badge，例如 `↑ 2/10 37%` 或 `↓ 2/10 37%`；这样长任务可以在后台继续，而不会长期占满整条底栏。真正断开当前连接时，队列会被清空。

普通文件现在支持最小版断点续传：再次上传或下载同一路径时，若目标侧已存在且大小小于等于源文件，就从该字节偏移继续；若目标侧大小反而更大，则回退为从 0 重新传输。这一版不做哈希校验，也不做跨重启的续传记录。

---

## 认证方式

`sush` 按以下顺序尝试认证：

1. **ssh-agent** — 若 `SSH_AUTH_SOCK` 已设置，优先使用
2. **IdentityFile** — 读取 `~/.ssh/config` 中的密钥路径；若私钥口令已保存在系统钥匙串，则优先读取，否则在 TUI 弹出输入框
3. **密码认证** — 所有方法失败后，优先读取系统钥匙串中的登录密码；取不到时再在 TUI 弹出输入框

首次手工输入成功后，`sush` 会静默尝试把以下凭证保存到系统原生安全存储：

- 登录密码
- 私钥口令

不同系统使用各自的原生能力：

- macOS：Keychain
- Windows：Credential Manager
- Linux：Secret Service

如果 Linux 环境没有可用的 Secret Service，`sush` 不会回退到本地加密文件；凭证只用于当前连接，并在下次需要输入时提示安装 `gnome-keyring` 或 `kwallet`。

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
| **v0.3** ✅ | TUI 主机编辑 · chip 标签编辑器 · 手动 SSH config 导入 |
| **v0.4** ✅ | 连接历史记录 · 近期加权搜索排序 · TCP 连通性探测 |
| **v0.5** ✅ | path 类型标签 · 主界面目录栏 · 目录跳转 · `path:` 范围搜索 |
| **v0.6** ✅ | 系统钥匙串凭证存储 · 认证成功后静默保存 · Linux 无 Secret Service 时仅临时输入 |
| **v0.7** ✅ | 文件夹递归传输与聚合进度 · 远程文件编辑并保存后自动上传 · 双面板 SFTP · 后台传输队列 · 断点续传 |
| **v0.8** ✅ | 端口转发管理 · 单跳 ProxyJump · SOCKS5 代理 · 转发状态视图 |
| v1.0 | macOS smoke test · GitHub Actions 六平台二进制发布 · 文档一致性 |

---

## 技术栈

- [russh](https://github.com/Eugeny/russh) — 纯 Rust SSH 实现，无 C 依赖
- [alacritty_terminal](https://github.com/alacritty/alacritty) — VT100/xterm 终端模拟器
- [ratatui](https://ratatui.rs) — TUI 渲染框架
- [nucleo](https://github.com/helix-editor/nucleo) — 模糊搜索（Helix 编辑器同款）
- [tokio](https://tokio.rs) — 异步运行时

单文件二进制，无 `libssh2`。Linux 若要持久化凭证，需要系统提供 Secret Service。

---

## 许可证

MIT
