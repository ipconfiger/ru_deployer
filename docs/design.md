# ru_deployer 设计文档

## 1. 项目目标

用 Rust 重写 `listen_push.py`（`~/Projects/tools/listen_push.py`），并将部分原本由 shell 脚本执行的功能集成到 Rust 中，减少对外部脚本的依赖，提高可靠性和可维护性。

### 1.1 保留的功能（继承自 listen_push.py）

- 轮询 GitLab REST API，检测 push 事件
- 四种监听模式：`commits`、`events`、`global`、`multi`
- 项目/分支过滤器（原 `listen_filter.json`）
- 调用外部部署脚本（`*_deploy.sh`）
- 部署结果邮件通知

### 1.2 新增/集成的功能（从 shell 脚本移入 Rust）

| 功能 | 原实现 | Rust 实现 |
|---|---|---|
| Git 代码拉取 | `*_deploy.sh` 中的 `git fetch/clone` | Rust 内调用 `git2` 或 `git` CLI |
| 邮件通知 | `listen_push.py` 调用消息平台 API | Rust 内 HTTP 客户端直接调用 |
| 脚本输出捕获 | shell `capture_output` | Rust `Command` stdout/stderr 捕获，错误时附到邮件 |

### 1.3 保留在外部脚本的功能（新建于 `scripts/`，不修改原脚本）

- Docker 镜像编译（`docker build`）
- Harbor 镜像推送（`push_harbor.sh`）
- 容器重启（`docker compose up -d`）

---

## 2. 架构概览

```
ru_deployer
├── Cargo.toml
├── config.toml              # 主配置文件
├── filter.toml              # 项目/分支过滤器
├── scripts/                 # 外部部署脚本（从 tools/ 复制）
│   ├── api_deploy.sh
│   ├── flint_deploy.sh
│   ├── horizon_deploy.sh
│   └── api-manager-platform_deploy.sh
├── src/                     # Git 拉取的源码存放目录
│   └── <project>/
│   └── <branch>/
├── data/                    # SQLite 数据库文件目录
│   └── deploy.db
└── src/                     # Rust 源码
    ├── main.rs              # 入口，CLI 解析，模式分发
    ├── config.rs            # 配置加载（TOML + 环境变量覆盖）
    ├── filter.rs            # 事件过滤器
    ├── gitlab.rs            # GitLab API 客户端
    ├── git.rs               # Git 操作（clone / fetch / checkout）
    ├── deploy.rs            # 部署脚本执行与输出捕获
    ├── notify.rs            # 邮件通知（消息平台 API）
    ├── db.rs                # SQLite 部署历史存储
    └── logging.rs           # 日志初始化（tracing）
```

### 2.1 数据流

```
                    ┌─────────────────────────┐
                    │      GitLab Server       │
                    └───────────┬─────────────┘
                                │ REST API (polling)
                    ┌───────────▼─────────────┐
                    │      gitlab.rs           │
                    │  Events / Commits API    │
                    └───────────┬─────────────┘
                                │ push event
                    ┌───────────▼─────────────┐
                    │      filter.rs           │
                    │  project + branch match  │
                    └───────────┬─────────────┘
                                │ matched
                    ┌───────────▼─────────────┐
                    │       git.rs             │
                    │  clone / fetch           │
                    │  → src/<project>/<branch>│
                    └───────────┬─────────────┘
                                │ code ready
                    ┌───────────▼─────────────┐
                    │      deploy.rs           │
                    │  exec *_deploy.sh        │
                    │  capture stdout/stderr   │
                    │  collect exit code       │
                    └───────────┬─────────────┘
                                │ result
                    ┌───────────▼─────────────┐
                    │      notify.rs           │
                    │  成功 → 简要通知          │
                    │  失败 → 附带错误日志       │
                    └───────────┬─────────────┘
                                │
                    ┌───────────▼─────────────┐
                    │        db.rs             │
                    │  INSERT deployment       │
                    │  → data/deploy.db        │
                    └─────────────────────────┘
```

---

## 3. 配置设计

### 3.1 `config.toml`（主配置）

```toml
[gitlab]
url = "http://172.16.29.88:30080"
token = "glpat-xxxxxxxxxxxxxxxx"
poll_interval_secs = 10

[deploy]
# 模式: "multi" | "events" | "global" | "commits"
mode = "multi"
# 部署脚本目录
scripts_dir = "./scripts"
# Git 源码存放根目录
src_dir = "./src"
# commits 模式专用: 监控分支
branch = "main"

[filter]
# 过滤器文件路径（mode=multi 时必填）
file = "./filter.toml"

[notify]
# 消息平台 API 地址
msg_platform_url = "http://172.16.42.213:8081"
# 收件人取 author email（默认 true）
notify_author = true
# 额外抄送列表
cc = []

[logging]
level = "info"
# 日志输出: "stdout" | "file"
output = "stdout"
# output=file 时的路径
file = "/var/log/ru_deployer.log"

[database]
# SQLite 数据库文件路径
path = "./data/deploy.db"
```

### 3.2 `filter.toml`（过滤器）

```toml
[[repos]]
project = "dev-team/api"
branches = ["main", "mysql_db"]

[[repos]]
project = "dev-team/flint"
branches = ["main", "develop"]

[[repos]]
project = "dev-team/horizon"
branches = ["feat/multi"]

[[repos]]
project = "dev-team/api-manager-platform"
branches = ["feature/bare-metal-rental"]
```

> `branches = []` 表示该项目所有分支都放行。

### 3.3 环境变量覆盖

所有配置项均可通过环境变量覆盖，遵循 `RU_DEPLOYER_<SECTION>_<KEY>` 格式：

```bash
export RU_DEPLOYER_GITLAB_TOKEN="glpat-xxx"
export RU_DEPLOYER_GITLAB_POLL_INTERVAL_SECS="5"
export RU_DEPLOYER_NOTIFY_MSG_PLATFORM_URL="http://..."
```

也可以通过 `RU_DEPLOYER_CONFIG` 指定配置文件路径（默认 `./config.toml`）。

---

## 4. 模块详细设计

### 4.1 `gitlab.rs` — GitLab API 客户端

**职责**: 封装 GitLab REST API 调用，负责事件轮询。

```rust
pub struct GitLabClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl GitLabClient {
    /// 查询项目最新 events（action=pushed）
    pub async fn get_project_events(&self, project_path: &str, per_page: u32) -> Result<Vec<Event>>;

    /// 查询全局 events
    pub async fn get_global_events(&self, per_page: u32) -> Result<Vec<Event>>;

    /// 查询分支最新 commits
    pub async fn get_commits(&self, project_path: &str, branch: &str, per_page: u32) -> Result<Vec<Commit>>;

    /// 通过 project_id 查询项目路径
    pub async fn lookup_project(&self, project_id: u64) -> Result<String>;

    /// 通过 user_id 查询用户邮箱
    pub async fn get_user_email(&self, user_id: u64) -> Result<String>;
}
```

**事件轮询策略**（各模式共享）:
- 维护 `last_event_id`（或 `last_commit_sha`）
- 每次轮询比较最新记录 ID
- 变化时触发 `deploy::handle_push`

### 4.2 `filter.rs` — 过滤器

```rust
pub struct Filter {
    repos: Vec<RepoRule>,
}

struct RepoRule {
    project: String,
    branches: Vec<String>,  // 空 = 该项目全部放行
}

impl Filter {
    pub fn load(path: &Path) -> Result<Self>;
    pub fn is_empty(&self) -> bool;  // 无过滤器 = 全放行
    pub fn matches(&self, project: &str, branch: &str) -> bool;
}
```

匹配规则（与 Python 版一致）：
1. `filter.is_empty()` → 放行所有
2. 项目不在 `repos` 中 → 拒绝
3. `branches` 为空 → 该项目全部分支放行
4. `branches` 非空 → 精确匹配分支名

### 4.3 `git.rs` — Git 操作

**职责**: 将部署脚本中的 git clone/fetch 逻辑移入 Rust。

```rust
pub struct GitRepo {
    src_dir: PathBuf,       // ./src
}

impl GitRepo {
    /// 确保本地有指定项目+分支的最新代码
    /// 目录结构: src/<project>/<branch>/
    pub async fn ensure(
        &self,
        gitlab_url: &str,
        token: &str,
        project: &str,     // e.g. "dev-team/api"
        branch: &str,      // e.g. "main"
    ) -> Result<PathBuf>;  // 返回仓库本地路径

    /// 获取当前 HEAD commit SHA
    pub fn current_commit(&self, repo_path: &Path) -> Result<String>;
}
```

**实现策略**:

选择 **调用系统 `git` CLI**（而非 `git2` 库）：

| 方案 | 优点 | 缺点 |
|---|---|---|
| `git2` (libgit2) | 纯 Rust，无外部依赖 | API 复杂，认证支持弱，clone 行为不够标准 |
| `git` CLI | 行为一致，认证简单，兼容性好 | 依赖系统安装 git |

**推荐 `git` CLI**，原因：
- 需要 `git clone --depth 1` 浅克隆减少网络开销
- OAuth2 token 认证通过 URL 直接搞定
- 与现有脚本行为 100% 一致

**工作流**:

```
ensure(project="dev-team/api", branch="main")
  → target_dir = src/api/main/
  → if .git 存在:
      git fetch origin main --depth 1
      git checkout main
      git reset --hard origin/main
  → else:
      git clone --depth 1 --branch main \
        http://oauth2:<token>@gitlab/dev-team/api.git \
        src/api/main/
  → return target_dir
```

> 认证 URL: `http://oauth2:{GITLAB_TOKEN}@{GITLAB_HOST}/{project}.git`

### 4.4 `deploy.rs` — 部署执行

**职责**: 编排部署流程，执行外部脚本，收集输出。

```rust
pub struct Deployer {
    scripts_dir: PathBuf,
}

pub struct DeployResult {
    pub exit_code: i32,
    pub stdout: String,       // 限制最大长度（如 100KB）
    pub stderr: String,       // 限制最大长度
    pub commit: String,       // 当前 commit SHA
    pub duration: Duration,
}

impl Deployer {
    /// 执行完整部署流程
    pub async fn deploy(&self, event: &PushEvent) -> Result<DeployResult> {
        // 1. git ensure → 拉代码到 src/<project>/<branch>
        // 2. 执行 <project>_deploy.sh（带环境变量）
        // 3. 收集结果
    }
}
```

**脚本执行环境变量**（与 Python 版兼容）：

| 变量 | 值 |
|---|---|
| `GITLAB_PROJECT` | `dev-team/api` |
| `GITLAB_BRANCH` | `main` |
| `GITLAB_COMMIT` | 完整 SHA |
| `GITLAB_AUTHOR` | 提交者用户名 |
| `GITLAB_EVENT_ID` | 事件 ID |
| `SRC_DIR` | `src/<project>/<branch>` (新增，脚本可直接用) |

**并发控制**: 同一项目同一分支的部署串行化（`DashMap<Key, Mutex<()>>`），避免同一事件触发多次部署。

### 4.5 `notify.rs` — 邮件通知

**职责**: 通过消息平台 API 发送 HTML 邮件。

```rust
pub struct Notifier {
    api_url: String,
    client: reqwest::Client,
}

impl Notifier {
    /// 发送部署结果邮件
    pub async fn notify(&self, event: &PushEvent, result: &DeployResult, author_email: &str);
}
```

**邮件模板**:

- **成功**: 项目、分支、commit、时间，绿色标题
- **失败**: 同上 + 红色标题 + 错误日志（`stderr` + `stdout` 末尾 3000 字符），深色背景 `<pre>` 块
- 邮件主题: `[✅/❌] <project>@<branch> 部署<成功/失败>`

**API 调用**（兼容现有消息平台）:

```json
POST /api/v1/mail/send_raw
{
  "to": ["user@example.com"],
  "subject": "[✅] api@main 部署成功",
  "content_type": "html",
  "content": "<h2>✅ 部署成功</h2>..."
}
```

### 4.6 `db.rs` — SQLite 部署历史

**职责**: 持久化每次部署的完整记录，支持查询历史。

```rust
pub struct DeploymentDb {
    pool: SqlitePool,
}

/// 单次部署记录
pub struct DeploymentRecord {
    pub id: i64,                    // 自增主键
    pub project: String,            // e.g. "dev-team/api"
    pub branch: String,             // e.g. "main"
    pub commit_sha: String,         // 完整 SHA
    pub author_name: String,        // GitLab 用户名
    pub author_email: String,       // GitLab 用户邮箱
    pub event_id: u64,              // GitLab event ID
    pub exit_code: i32,             // 部署脚本退出码 (0=成功)
    pub status: DeploymentStatus,   // 成功 / 失败
    pub stdout_tail: String,        // stdout 末尾 (限 10KB)
    pub stderr_tail: String,        // stderr 末尾 (限 10KB)
    pub duration_ms: i64,           // 部署耗时 (毫秒)
    pub created_at: DateTime<Utc>,  // 部署时间
}

pub enum DeploymentStatus {
    Success,
    Failed,
}

impl DeploymentDb {
    /// 打开/创建数据库，自动执行 migration
    pub async fn open(path: &Path) -> Result<Self>;

    /// 插入一条部署记录
    pub async fn insert(&self, record: &DeploymentRecord) -> Result<i64>;

    /// 查询某项目的最近 N 次部署
    pub async fn recent(&self, project: &str, limit: u32) -> Result<Vec<DeploymentRecord>>;

    /// 查询某项目某分支的最近 N 次部署
    pub async fn recent_by_branch(&self, project: &str, branch: &str, limit: u32) -> Result<Vec<DeploymentRecord>>;

    /// 统计某项目的部署成功率
    pub async fn stats(&self, project: &str, days: u32) -> Result<DeployStats>;
}

pub struct DeployStats {
    pub total: u64,
    pub success: u64,
    pub failed: u64,
}
```

**数据库 Schema**:

```sql
CREATE TABLE IF NOT EXISTS deployments (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    project      TEXT    NOT NULL,    -- "dev-team/api"
    branch       TEXT    NOT NULL,    -- "main"
    commit_sha   TEXT    NOT NULL,    -- 完整 40-char SHA
    author_name  TEXT    NOT NULL,
    author_email TEXT    NOT NULL DEFAULT '',
    event_id     INTEGER NOT NULL,    -- GitLab event ID
    exit_code    INTEGER NOT NULL,
    status       TEXT    NOT NULL CHECK (status IN ('success', 'failed')),
    stdout_tail  TEXT    NOT NULL DEFAULT '',   -- 末尾 10KB
    stderr_tail  TEXT    NOT NULL DEFAULT '',   -- 末尾 10KB
    duration_ms  INTEGER NOT NULL,
    created_at   TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_deployments_project ON deployments(project);
CREATE INDEX IF NOT EXISTS idx_deployments_project_branch ON deployments(project, branch);
CREATE INDEX IF NOT EXISTS idx_deployments_created_at ON deployments(created_at);
```

**设计要点**:
- 输出截断：`stdout`/`stderr` 仅保留末尾 10KB，避免数据库膨胀
- 时间使用 ISO 8601 文本存储，SQLite 无原生 datetime 类型
- Migration 在 `open()` 时自动执行，使用 `sqlx::migrate!` 或手动 DDL
- 历史查询接口为日后回滚功能提供数据基础（回滚方案待定）

---


```bash
# 默认 multi 模式
ru_deployer

# 指定模式
ru_deployer --mode multi
ru_deployer --mode events
ru_deployer --mode global
ru_deployer --mode commits --branch main

# 指定配置文件
ru_deployer --config /etc/ru_deployer/config.toml

# 指定 filter 文件（覆盖 config.toml 中的设置）
ru_deployer --filter ./filter.toml
```

**信号处理**: `SIGTERM` / `SIGINT` 优雅退出，完成当前部署轮次后退出。

---

## 6. 部署脚本

### 6.1 原则

**不修改** `~/Projects/tools/` 下的任何现有脚本。在 `ru_deployer/scripts/` 下新建脚本，逻辑可参考原脚本。

### 6.2 脚本职责

1. `docker build` — 构建逻辑与项目强耦合，保留
2. `push_harbor.sh` — 独立工具，保留
3. `docker compose up -d` — 编排逻辑，保留

以下功能已移入 Rust：
- `git fetch/clone` → `git.rs`（Rust 拉代码到 `src/<project>/<branch>/`，通过 `SRC_DIR` 环境变量传给脚本）

### 6.3 新脚本模板

```bash
#!/bin/bash
set -e

# Rust 传入的环境变量:
#   GITLAB_PROJECT   e.g. "dev-team/api"
#   GITLAB_BRANCH    e.g. "main"
#   GITLAB_COMMIT    完整 SHA
#   GITLAB_AUTHOR    提交者用户名
#   SRC_DIR          Rust 已拉好的代码目录 (e.g. /opt/ru_deployer/src/api/main)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_NAME="${GITLAB_PROJECT##*/}"

echo "开始部署 ${GITLAB_PROJECT} @ ${GITLAB_BRANCH}"

cd "${SRC_DIR}"
ACTUAL_COMMIT=$(git rev-parse --short HEAD)

# docker build
IMAGE_NAME="${PROJECT_NAME}:latest"
docker build -t "${IMAGE_NAME}" -f "<Dockerfile相对路径>" .

# push harbor
"${SCRIPT_DIR}/push_harbor.sh" "${IMAGE_NAME}" "gpu_${PROJECT_NAME}_dev" "${ACTUAL_COMMIT}"

# restart
cd "${SCRIPT_DIR}/.." && docker compose up -d "${PROJECT_NAME}"

echo "部署完成 (commit: ${ACTUAL_COMMIT})"
```

### 6.4 与原脚本的差异

| 项目 | 原脚本 (`tools/`) | 新脚本 (`ru_deployer/scripts/`) |
|---|---|---|
| Git 拉取 | 脚本内 `git fetch/clone` | Rust 已完成，通过 `SRC_DIR` 传入 |
| REPO_DIR | `"${SRC_DIR}/${PROJECT_NAME}"` | 直接使用 `"${SRC_DIR}"` |
| GitLab 认证 token | 硬编码在脚本中 | 无需（Rust 处理） |
| Dockerfile registry 替换 | sed 替换外网地址 | 保留 |
| docker build / push / up | 保留 | 保留 |

---

## 7. 依赖 crate

| crate | 用途 |
|---|---|
| `tokio` | 异步运行时 |
| `reqwest` | HTTP 客户端（GitLab API + 邮件 API） |
| `serde` / `serde_json` | JSON 序列化 |
| `toml` | TOML 配置解析 |
| `clap` | CLI 参数解析 |
| `tracing` / `tracing-subscriber` | 结构化日志 |
| `dashmap` | 并发安全的部署锁 |
| `sqlx` (feature: `sqlite`, `runtime-tokio`) | SQLite 数据库操作 |
| `chrono` | 时间处理 |
| `anyhow` / `thiserror` | 错误处理 |
| `tokio::process::Command` | 执行外部脚本（std 已包含） |

---

## 8. 生产部署

### 8.1 systemd 服务

```
# /etc/systemd/system/ru_deployer.service
[Unit]
Description=ru_deployer - GitLab Push Event Watcher & Deployer
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
WorkingDirectory=/opt/ru_deployer
ExecStart=/opt/ru_deployer/ru_deployer --mode multi --config /opt/ru_deployer/config.toml
Restart=always
RestartSec=5
StandardOutput=append:/var/log/ru_deployer.log
StandardError=append:/var/log/ru_deployer.log

[Install]
WantedBy=multi-user.target
```

### 8.2 目录布局

```
/opt/ru_deployer/
├── ru_deployer           # 编译好的二进制
├── config.toml            # 配置文件
├── filter.toml            # 过滤器
├── data/                  # 运行时数据
│   └── deploy.db          # SQLite 部署历史
├── scripts/               # 部署脚本
│   ├── api_deploy.sh
│   ├── flint_deploy.sh
│   ├── horizon_deploy.sh
│   ├── api-manager-platform_deploy.sh
│   └── push_harbor.sh
└── src/                   # Git 拉取的工作目录
    ├── api/
    │   ├── main/
    │   └── mysql_db/
    ├── flint/
    │   ├── main/
    │   └── develop/
    ├── horizon/
    │   └── feat/multi/
    └── api-manager-platform/
        └── feature/bare-metal-rental/
```

---

## 9. 决策记录

| # | 议题 | 决策 |
|---|---|---|
| 1 | git 实现方案 | **系统 `git` CLI**，简单简洁是第一原则 |
| 2 | 配置文件格式 | **TOML**，支持注释，可读性更好 |
| 3 | 部署脚本 | **不修改现有脚本**。在本项目 `scripts/` 下新建脚本，仅使用 Rust 传入的环境变量（不自行计算 `REPO_DIR`，改用 `SRC_DIR`） |
| 4 | Webhook 支持 | **不支持**，保持简洁 |
| 5 | 多实例支持 | **暂不需要** |
| 6 | 部署历史 | **SQLite** 存储，`data/deploy.db` |
| 7 | 回滚功能 | **待定**，历史数据已留存，后续可基于此实现 |

### 9.1 新建脚本适配说明

现有 `~/Projects/tools/*_deploy.sh` 不做任何修改。在 `ru_deployer/scripts/` 下创建新的部署脚本，唯一区别是：

```bash
# 旧脚本：自行拼路径
REPO_DIR="${SRC_DIR}/${PROJECT_NAME}"

# 新脚本：直接用 Rust 传入的 SRC_DIR
REPO_DIR="${SRC_DIR}"
```

其余逻辑（docker build、push_harbor.sh、docker compose up）保持一致，可直接参考原脚本。
