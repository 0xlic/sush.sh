# sush.sh (sush) 需求文档索引

## 说明

需求文档已按 `ROADMAP.md` 中的版本拆分到 `docs/requirements/` 目录下，每个版本对应一个独立文件。

这些版本文档采用按版本增量描述的方式：
- `v0.1` 记录 MVP 基础范围
- `v0.2` 及后续版本记录相对于前一版本的新增或调整需求
- `v1.0` 记录稳定发布阶段的验收要求

`docs/REQUIREMENTS.md` 继续保留为入口索引，不再承载完整需求正文。

## 版本索引

| 版本 | 文档 | 主题 |
|------|------|------|
| v0.1 | [v0.1](requirements/v0.1.md) | MVP：SSH、SFTP、基础传输 |
| v0.2 | [v0.2](requirements/v0.2.md) | 嵌入式终端模式 |
| v0.3 | [v0.3](requirements/v0.3.md) | 主机管理 CRUD |
| v0.4 | [v0.4](requirements/v0.4.md) | 连接历史与连通性探测 |
| v0.5 | [v0.5](requirements/v0.5.md) | 虚拟目录导航 |
| v0.6 | [v0.6](requirements/v0.6.md) | 系统安全存储 |
| v0.7 | [v0.7](requirements/v0.7.md) | 完善文件操作 |
| v0.8 | [v0.8](requirements/v0.8.md) | 高级网络功能 |
| v1.0 | [v1.0](requirements/v1.0.md) | 稳定发布 |

## 使用约定

- 修改或新增某个版本功能时，优先更新对应版本文件
- 发版时按当前版本更新对应的 `docs/requirements/vX.Y.md`
- `superpowers` 目录中的计划 / 设计过程文档不是事实来源，不替代这里的版本需求文档
