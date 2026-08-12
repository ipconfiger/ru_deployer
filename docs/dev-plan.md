# ru_deployer 开发计划

## 阶段总览

```
Phase 1 (骨架+types) ──→ Phase 2 (filter + gitlab) ──→ Phase 6a (multi 模式)
                       ├─→ Phase 3 (git)          ──┘        │
                       └─→ Phase 4 (deploy+notify) ──┘        │
                               └─→ Phase 5 (db)    ───────────┘
                                                                │
                                                Phase 6b (其余模式) ──→ Phase 7a (简单脚本) ──→ Phase 7b (复杂脚本+部署) ──→ 完成
```

## Phase 1: 项目骨架 + 共享类型（无依赖）

**目标**: 可编译运行的空壳，配置和日志就绪，共享类型定义完成

**产出文件**:
- `Cargo.toml` — 依赖声明（含 `dotenvy` 用于 .env 加载）
- `src/main.rs` — CLI 解析（clap）、模式分发骨架（打印所选模式后退出）
- `src/config.rs` — `Config` 结构体、`load()` 函数（TOML + env 覆盖 + .env）
- `src/types.rs` — `Event`、`Commit`、`PushData`、`PushEvent` 共享类型（serde）
- `src/logging.rs` — `tracing-subscriber` 初始化
- `config.toml` — 示例配置（含 `[harbor]`、`[database]`、`script_timeout_secs` 等完整字段）
- `filter.toml` — 示例过滤器

**验收**: `cargo run -- --mode multi` 打印 "Mode: multi" 并退出；配置加载成功

---

## Phase 2: 过滤器 + GitLab 客户端（依赖 Phase 1）

**目标**: 过滤器加载/匹配可用，GitLab API 调用可用

**产出文件**:
- `src/filter.rs` — `Filter::load(path)`, `Filter::matches(project, branch)`
  - 内置 `refs/heads/` 前缀 stripping
  - 启动验证：multi 模式下探测 filter 中所有项目是否存在（WARNING 但不阻塞）
- `src/gitlab.rs` — `GitLabClient` struct，实现：
  - `get_project_events(project_path, per_page)`
  - `get_global_events(per_page)`
  - `get_commits(project_path, branch, per_page)`
  - `lookup_project(project_id)`
  - `get_user_email(user_id)`

> 共享类型 `Event`、`Commit`、`PushEvent` 等已由 Phase 1 在 `types.rs` 中定义

**验收**: 单元测试（filter 匹配规则含 refs/heads/ stripping、API mock 测试）

---

## Phase 3: Git 操作（依赖 Phase 1，独立于 Phase 2）

**目标**: 能通过 `git` CLI 拉取代码到指定目录

**产出文件**:
- `src/git.rs` — `GitRepo::ensure(url, token, project, branch) -> PathBuf`
  - 目录: `src/<short_name>/<branch>/`（short_name = project 最后一段，如 "dev-team/api" → "api"）
  - 首次: `git clone --depth 1 --branch <branch> <auth_url> <target>`
  - 后续: `git fetch origin <branch> --depth 1` + `checkout` + `reset --hard`
  - `current_commit(path) -> String`
  - 分支名安全性检查（拒绝含 `..` 的分支名，防止路径穿越）

**验收**: 手动指定一个项目+分支，验证 `src/api/main/` 下有正确代码

---

## Phase 4: 部署执行 + 邮件通知（依赖 Phase 1, 2, 3）

> 依赖 Phase 2 的共享类型（`types.rs` 中的 `PushEvent`）

**目标**: 能执行部署脚本并发送邮件

**产出文件**:
- `src/deploy.rs` — `Deployer::deploy(event) -> DeployResult`
  - 调用 `git.ensure()` 拉代码
  - 执行 `scripts/<project>_deploy.sh`（传入完整环境变量，含 `HARBOR_PASSWORD`、`SCRIPTS_DIR`）
  - `tokio::time::timeout` 超时控制（默认 1800s）
  - 捕获 stdout/stderr（头 2KB + 尾 8KB 截断策略）
  - 并发锁：`tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>` 方案
- `src/notify.rs` — `Notifier::notify(event, result, email)`
  - 成功 HTML 模板
  - 失败 HTML 模板（含错误日志：stderr 头 2KB + stdout 尾 8KB）

**验收**: 手动构造一个 PushEvent，验证脚本被执行、邮件 API 被调用（mock）

---

## Phase 5: SQLite 部署历史（依赖 Phase 1, 4 的 DeployResult 类型）

**目标**: 部署记录持久化到 SQLite

**产出文件**:
- `src/db.rs` — `DeploymentDb` struct
  - `open(path)` — 自动建表 migration（手动 DDL `CREATE TABLE IF NOT EXISTS`）+ WAL 模式
  - `insert(record)` — 写入一条记录
  - `recent(project, limit)` — 查询最近 N 次
  - `recent_by_branch(project, branch, limit)`
  - `stats(project, days)` — 成功率统计

**验收**: 集成测试 — insert 一条记录后 recent 查询能返回

---

## Phase 6a: multi 轮询模式（依赖 Phase 1-5）

**目标**: 先实现最核心的 multi 模式，端到端验证架构

**实现**:
- `src/modes/multi.rs` — 多项目独立轮询
  - 每个项目独立 `last_event_id`
  - filter 匹配 → git pull → 执行脚本 → 写 DB → 发邮件
  - 启动时验证 filter 中所有项目可达
  - 每 30 次轮询打印心跳（含各项目当前 last_event_id）
- `main.rs` — 模式分发，信号处理（`SIGTERM`/`SIGINT` 优雅退出）

**验收**: 端到端测试（mock GitLab API），验证 multi 模式完整链路

---

## Phase 6b: 其余三种模式（依赖 Phase 6a）

**实现**:
- `src/modes/events.rs` — 单项目 events 轮询（需 `[deploy].project` 配置）
- `src/modes/global.rs` — 全局 events 轮询
- `src/modes/commits.rs` — **仅监控**，比较最新 commit SHA，打印变更信息，不触发部署

**验收**: 各模式单元测试

---

## Phase 7a: 简单部署脚本（依赖 Phase 6a）

**目标**: 移植逻辑简单的脚本（api、flint），与 Rust 集成

**产出文件**:
- `scripts/api_deploy.sh` — 参考 `tools/api_deploy.sh`，移除 git 操作，使用 `SRC_DIR`
- `scripts/flint_deploy.sh` — 同上
- `scripts/push_harbor.sh` — 从 `tools/` 复制
- `scripts/docker-compose.yml` — 从 `tools/` 复制（或精简版仅含四个项目）
- `.env.example` — 环境变量模板

**验收**: 手动执行脚本（设置好环境变量），验证 docker build + push + up 全流程

---

## Phase 7b: 复杂部署脚本 + systemd + 收尾（依赖 Phase 7a）

**目标**: 移植逻辑复杂的脚本（horizon、api-manager-platform），生产就绪

**产出文件**:
- `scripts/horizon_deploy.sh` — 参考 `tools/horizon_deploy.sh`
  - 注意保留 `--build-arg VITE_*` 等关键参数
- `scripts/api-manager-platform_deploy.sh` — 参考 `tools/api-manager-platform_deploy.sh`
  - 注意保留 `npm ci`/`npm build`、Go 编译、config 修改等步骤
- `ru_deployer.service` — systemd unit（含 `EnvironmentFile` 支持从 `.env` 注入敏感变量）
- `README.md` — 部署说明

**验收**: 在目标机器上以 systemd 服务运行，触发一次真实部署
