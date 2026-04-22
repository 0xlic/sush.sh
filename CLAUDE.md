# sush.sh (sush) — 项目约束

## 项目简介

轻量级终端 SSH + SFTP 统一管理工具，TUI 界面，单文件分发。

## 语言环境
除了 `CLAUDE.md` 和 `doc` 目录使用中文，其他所有文件使用英文，包括commit message。

## 开发环境

- Rust 1.95 (edition 2024)，通过 mise 管理，版本锁定在 `mise.toml`
- 构建：`cargo build`
- 检查：`cargo check`
- 测试：`cargo test`
- 格式化：`cargo fmt`
- Lint：`cargo clippy -- -D warnings`

## 架构约束

### 核心文档

修改架构或新增模块前，必须先阅读对应文档：

- `REQUIREMENTS.md` — 功能需求与交互规则
- `ARCHITECTURE.md` — 技术栈、模块设计、数据流
- `ROADMAP.md` — 版本规划

### 技术选型（已锁定）

- SSH：russh 0.60（纯 Rust，无 C 依赖）
- SFTP：russh-sftp 2.1
- TUI：ratatui 0.30 + crossterm 0.29
- 异步：tokio 1.52
- 模糊搜索：nucleo 0.5
- 错误处理：anyhow 1.0（MVP 阶段）
- 配置格式：TOML

不要引入未在 `Cargo.toml` 中声明的新依赖，除非先讨论。

### 模块边界

```
src/
├── app.rs           # 状态机，模式切换逻辑
├── config/          # 主机配置、SSH Config 导入、TOML 持久化
├── ssh/             # SSH 连接、认证、会话管理
├── sftp/            # SFTP 操作、文件传输
├── tui/             # UI 渲染、事件循环、视图、组件
│   ├── views/       # 完整页面视图（主界面、SFTP 浏览器）
│   └── widgets/     # 可复用 UI 组件（搜索框、列表、进度条、状态栏）
└── utils/           # 工具函数（模糊搜索等）
```

新功能放入对应模块，不要在 `main.rs` 堆逻辑。

### 关键设计决策

- SSH 使用**接管模式**（I/O 转发 + 前缀键），不是内嵌终端模拟器
- 前缀键是 `Ctrl-Space`（raw mode 下字节 `0x00`），用于 SSH ↔ SFTP 切换
- SSH 和 SFTP **共享同一 TCP 连接**（不同 channel）
- SFTP 是**单面板**，Tab 切换本地/远程
- 双 `Ctrl-C` 仅在 SFTP/主界面生效，SSH 模式下 Ctrl-C 透传给远程
- 标签系统是**扁平标签**，没有分组层级

## 代码规范

### Rust 风格

- 优先函数式写法（迭代器链、map/filter/collect），避免不必要的 mut
- 错误传播用 `?` + anyhow，不要 `unwrap()`/`expect()` 除非在 100% 安全的上下文
- 类型定义加 `#[derive(Debug, Clone)]`，需要序列化的加 `Serialize, Deserialize`
- 公共 API 加 `pub`，内部实现不���露
- 每个模块的 `mod.rs` 只做 re-export，不放业务逻辑

### 不要做的事

- 不要引入 `unsafe` 代码
- 不要用 `println!` 做日志（TUI 模式下会破坏界面），如需调试用 `eprintln!` 或日志文件
- 不要在 TUI 渲染逻辑中做阻塞 I/O
- 不要硬编码路径，用 `dirs` crate 获取系统目录
- 不要把密码/密钥明文写入配置文件

## 验证清单

每次修改后确保：

1. `cargo check` 无错误
2. `cargo clippy -- -D warnings` 无警告
3. `cargo fmt --check` 格式正确
4. 如果改了 TUI 相关代码，手动运行 `cargo run` 验证界面表现
