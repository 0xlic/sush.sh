# v0.1 任务拆分

当前状态：目录结构和数据模型已搭好骨架，所有模块都是空壳/TODO。

## 第一层：基础设施（无 UI 依赖，可并行）

| # | 任务 | 模块 | 验证方式 |
|---|------|------|---------|
| **T1** | SSH Config 解析 | `config/ssh_config.rs` | 单元测试：解析真实 ssh config，返回正确的 Host 列表；跳过通配符；支持多 IdentityFile |
| **T2** | 配置持久化 + 增量同步 | `config/store.rs` | 单元测试：save → load 往返一致；hash 变更时增量更新（新增/更新，不删除 Manual 主机） |
| **T3** | 模糊搜索封装 | `utils/fuzzy.rs` | 单元测试：对 alias/hostname/user/tags 多维匹配，返回排序后的索引列表 |

## 第二层：TUI 框架（依赖 T1-T3）

| # | 任务 | 模块 | 验证方式 |
|---|------|------|---------|
| **T4** | TUI 事件循环 | `tui/event.rs` | `cargo run` 能启动 TUI，能响应键盘，`q` 退出不崩溃 |
| **T5** | 主界面渲染 | `views/main_view.rs` + `widgets/` | 搜索框 + 主机列表 + 底部快捷键栏正确渲染，上下键导航可选中 |
| **T6** | 搜索交互集成 | `app.rs` + `views/main_view.rs` | 输入字符实时过滤主机列表（调用 T3 的模糊搜索），空搜索显示全部 |

## 第三层：SSH 连接（依赖 T4-T6）

| # | 任务 | 模块 | 验证方式 |
|---|------|------|---------|
| **T7** | SSH 认证流程 | `ssh/auth.rs` | 实现 agent → key → password 三级认证链；密码输入需要 T8 |
| **T8** | 密码输入弹窗 | `views/password_dialog.rs` | 弹窗遮罩渲染，输入密码时显示 `*`，Enter 确认 / Esc 取消 |
| **T9** | SSH 接管模式 | `ssh/session.rs` + `app.rs` | Enter 连接主机 → raw mode I/O 转发 → 远程 shell 完全可用 → `exit`/`Ctrl-D` 返回主界面 |
| **T10** | 前缀键 + 终端 resize | `ssh/session.rs` | `Ctrl-Space`(0x00) 拦截触发模式切换；resize 事件转发给远程 PTY |

## 第四层：SFTP（依赖 T9-T10）

| # | 任务 | 模块 | 验证方式 |
|---|------|------|---------|
| **T11** | SFTP 客户端封装 | `sftp/client.rs` | 复用 SSH 连接开 SFTP channel；列目录、读文件信息正常返回 |
| **T12** | SFTP 文件浏览器 UI | `views/sftp_view.rs` + `widgets/file_list.rs` | 单面板渲染文件列表（目录在前、文件在后），Enter 进入目录，`..` 返回上级，Tab 切本地/远程 |
| **T13** | 文件上传/下载 + 进度 | `sftp/transfer.rs` + `widgets/progress_bar.rs` | F5 下载 / F6 上传单文件，底部进度条实时更新，完成后提示 |

## 第五层：收尾（依赖全部）

| # | 任务 | 模块 | 验证方式 |
|---|------|------|---------|
| **T14** | 双 Ctrl-C 返回 + 模式切换完善 | `app.rs` | SFTP 双 Ctrl-C 断连回主界面；SSH 模式 Ctrl-C 透传；主界面 Ctrl-C 清搜索/退出；传输中 Ctrl-C 取消传输 |
| **T15** | 端到端集成测试 | 全模块 | 完整走通：启动 → 搜索 → SSH 连接 → Ctrl-Space 切 SFTP → 上传下载 → 切回 SSH → exit 返回主界面 |

## 执行顺序

```
T1 + T2 + T3  (并行，纯逻辑无 UI)
      ↓
T4 → T5 → T6  (串行，逐步搭建 TUI)
      ↓
T7 + T8 → T9 → T10  (SSH 核心)
      ↓
T11 → T12 → T13  (SFTP)
      ↓
T14 → T15  (收尾)
```
