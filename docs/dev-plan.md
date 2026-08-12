# ru_deployer 开发计划

## 阶段总览

```
Phase 1 (骨架) ──→ Phase 2 (filter + gitlab) ──→ Phase 6 (轮询模式)
                 └─→ Phase 3 (git)          ──┘        │
                 └─→ Phase 4 (deploy+notify) ──┘        │
                         └─→ Phase 5 (db)    ───────────┘
                                                          │
                                              Phase 7 (脚本+部署) ──→ 完成
```

## Phase 1: 项目骨架（无依赖）

**目标**: 可编译运行的空壳，配置和日志就绪

**产出文件**:
- `Cargo.toml` — 依赖声明
- `src/main.rs` — CLI 解析（clap）、模式分发骨架（打印所选模式后退出）
- `src/config.rs` — `Config` 结构体、`load()` 函数（TOML + env 覆盖）
- `src/logging.rs` — `tracing-subscriber` 初始化
- `config.toml` — 示例配置
- `filter.toml` — 示例过滤器

**验收**: `cargo run -- --mode multi` 打印 "Mode: multi" 并退出；配置加载成功

---

## Phase 2: 过滤器 + GitLab 客户端（依赖 Phase 1）

**目标**: 过滤器加载/匹配可用，GitLab API 调用可用

**产出文件**:
- `src/filter.rs` — `Filter::load(path)`, `Filter::matches(project, branch)`
- `src/gitlab.rs` — `GitLabClient` struct，实现：
  - `get_project_events(project_path, per_page)`
  - `get_global_events(per_page)`
  - `get_commits(project_path, branch, per_page)`
  - `lookup_project(project_id)`
  - `get_user_email(user_id)`

**API 类型定义**: `Event`、`Commit`、`PushData`（serde Deserialize）

**验收**: 单元测试（filter 匹配规则、API mock 测试）

---

## Phase 3: Git 操作（依赖 Phase 1，独立于 Phase 2）

**目标**: 能通过 `git` CLI 拉取代码到指定目录

**产出文件**:
- `src/git.rs` — `GitRepo::ensure(url, token, project, branch) -> PathBuf`
  - 首次: `git clone --depth 1 --branch <branch> <auth_url> <target>`
  - 后续: `git fetch origin <branch> --depth 1` + `checkout` + `reset --hard`
  - `current_commit(path) -> String`

**验收**: 手动指定一个项目+分支，验证 `src/<project>/<branch>/` 下有正确代码

---

## Phase 4: 部署执行 + 邮件通知（依赖 Phase 1, 3，独立于 Phase 2）

**目标**: 能执行部署脚本并发送邮件

**产出文件**:
- `src/deploy.rs` — `Deployer::deploy(event) -> DeployResult`
  - 调用 `git.ensure()` 拉代码
  - 执行 `scripts/<project>_deploy.sh`（传入环境变量）
  - 捕获 stdout/stderr，限制大小
  - 并发锁（同项目同分支串行）
- `src/notify.rs` — `Notifier::notify(event, result, email)`
  - 成功 HTML 模板
  - 失败 HTML 模板（含错误日志）

**验收**: 手动构造一个 PushEvent，验证脚本被执行、邮件 API 被调用（mock）

---

## Phase 5: SQLite 部署历史（依赖 Phase 4 的 DeployResult 类型）

**目标**: 部署记录持久化到 SQLite

**产出文件**:
- `src/db.rs` — `DeploymentDb` struct
  - `open(path)` — 自动建表 migration
  - `insert(record)` — 写入一条记录
  - `recent(project, limit)` — 查询最近 N 次
  - `recent_by_branch(project, branch, limit)`
  - `stats(project, days)` — 成功率统计

**验收**: 集成测试 — insert 一条记录后 recent 查询能返回

---

## Phase 6: 四种轮询模式（依赖 Phase 1-5）

**目标**: 完整的部署自动化流程

**实现**:
- `src/modes/commits.rs` — 比较最新 commit SHA
- `src/modes/events.rs` — 单项目 events 轮询
- `src/modes/global.rs` — 全局 events 轮询
- `src/modes/multi.rs` — 多项目独立轮询（默认）

**统一轮询流程**: 检测变化 → filter 匹配 → git pull → 执行脚本 → 写 DB → 发邮件

**信号处理**: tokio `signal::ctrl_c()` 优雅退出

**验收**: 端到端测试（mock GitLab API），验证完整链路

---

## Phase 7: 部署脚本 + systemd + 收尾（依赖 Phase 6）

**目标**: 生产就绪

**产出文件**:
- `scripts/api_deploy.sh`
- `scripts/flint_deploy.sh`
- `scripts/horizon_deploy.sh`
- `scripts/api-manager-platform_deploy.sh`
- `scripts/push_harbor.sh`（从 tools 复制）
- `ru_deployer.service` — systemd unit
- `README.md` — 部署说明

**验收**: 在目标机器上以 systemd 服务运行，触发一次真实部署
