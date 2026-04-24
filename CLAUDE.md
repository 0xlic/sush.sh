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

每个新版本验收无误之后，需要更新上述文档及中英文README文档，以及打上git tag版本号

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
