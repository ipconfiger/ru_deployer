# ru_deployer 技术债与待办治理方案

**日期**: 2026-08-13
**分支**: master（工作区含未提交改动：dogress 脚本、oneshot 模式、4 个新 release 脚本）
**来源**: `docs/status.md` §技术债务/待办 + 本次代码审查新增发现

---

## 1. 背景与目标

`docs/status.md` 列出了 4 项技术债/待办：

| # | 项目 | 优先级 | 说明 |
|---|---|---|---|
| T1 | `push_harbor.sh` 参数统一 | 低 | release 脚本用 HARBOR_PASSWORD，deploy 脚本用 push_harbor.sh |
| T2 | DB 历史查询 CLI | 低 | 添加 `--show-recent` 等子命令查看部署历史 |
| T3 | ~~回滚功能~~ | **已决策：不实施** | 见 §3.1 T3 决策记录 |
| T4 | 日志轮转 | 低 | 当前依赖 systemd journal |

此外，本次对源码、脚本、文档的全面 review 发现 **7 项未记录的问题**（F1–F7，见 §3.2），其中 F1 为已实证的代码缺陷，建议与 T1–T4 一并治理。

本文档目标：为每项给出**具体方案、影响面、验收标准与阻塞评估**，供决策后实施。

---

## 2. 现状盘点（基于当前工作区代码）

### 2.1 关键事实（已实证）

- 构建：`cargo build` 通过（当前有 **1 个 dead_code 警告**：`ReleaseEvent::from_event` 未使用，见 F5/N4）；`cargo test` 25/25 通过。
- 脚本子进程注入的环境变量（`src/deploy.rs` `run_script`）：仅 `GIT_BRANCH` / `VERSION` / `HARBOR_PASSWORD` 三个。
- `push_harbor.sh` 内硬编码默认值：`HARBOR_REGISTRY=172.16.29.88:30800`、`HARBOR_PROJECT=gpu`、`HARBOR_USER=robot$gpu+gpubot`、**`HARBOR_PASSWORD` 有写死密码兜底**。
- `config.toml` `[harbor]` 段：`registry` / `project` / `user` / `password`（当前 `password=""`，依赖环境变量或脚本兜底）。
- `src/db.rs`：`recent()` / `recent_by_branch()` / `stats()` 已实现，标记 `#[allow(dead_code)]`，可直接复用。
- `src/git.rs` `ensure_tag()`：更新路径执行 `git fetch --tags --unshallow origin`，未判断仓库是否浅克隆。
- 主程序 CLI（`src/main.rs`）：扁平 `clap::Parser` 结构，已有 `--once --service` 动作参数。
- 日志输出：`StandardOutput=append:/var/log/ru_deployer.log`（systemd），无轮转。
- **已实证**：`git fetch --unshallow` 在完整仓库上返回 exit 128（`fatal: --unshallow on a complete repository does not make sense`）。

---

## 3. 待治理项

### 3.1 既有待办（T1–T4）

#### T1. push_harbor.sh 参数统一（建议 P1，先做）

**问题**：Harbor 配置存在三处来源且不一致 —— `config.toml [harbor]`、`.env`/环境变量、`push_harbor.sh` 内写死默认值。后果：
1. `config.toml` 中 `password=""` 时，Rust 注入空 `HARBOR_PASSWORD`，但脚本内 `${HARBOR_PASSWORD:-写死密码}` 兜底，**"无密码时静默跳过"的设计永不生效**；
2. 改 Harbor 地址/账号必须改脚本，与"配置驱动"原则相悖；
3. 凭据硬编码进 git 仓库（见 F6）。

**方案**（配置驱动，Rust 全量注入）：
1. `src/deploy.rs`：`Deployer` 结构体由 `harbor_password: String` 扩展为 `harbor: HarborConfig`（或 4 个独立字段），`run_script()` 增加注入 4 个环境变量：
   - `HARBOR_REGISTRY`、`HARBOR_PROJECT`、`HARBOR_USER`、`HARBOR_PASSWORD`
2. `push_harbor.sh` 修改：
   - **删除全部硬编码默认值**；改为 `: "${HARBOR_REGISTRY:?}"` 严格模式或保留 registry/project/user 默认（仅密码必须无默认）。
   - 密码为空 → 打印提示并 `exit 0`（恢复原设计语义）。
3. `config.rs`：`HarborConfig` 已有 4 字段；补全 `RU_DEPLOYER_HARBOR_REGISTRY` / `RU_DEPLOYER_HARBOR_PROJECT` / `RU_DEPLOYER_HARBOR_USER` 三个环境变量覆盖（当前仅 `RU_DEPLOYER_HARBOR_PASSWORD` 有实现）。
4. 清理：若确认不再需要，从仓库删除写死密码（F6）。
5. **兼容 plain `HARBOR_PASSWORD`（N3，实施前必须处理）**：`config.rs` 目前只读 `RU_DEPLOYER_HARBOR_PASSWORD`，而 `config.toml` 注释与 `docs/design.md` 声称 `HARBOR_PASSWORD` 环境变量可用——该路径**今天靠脚本写死密码兜底才"看似生效"**。删除兜底后，依赖 `.env`/`EnvironmentFile` 中 plain `HARBOR_PASSWORD` 的部署会**静默跳过推送**。二选一：(a) `config.rs` 补读 plain `HARBOR_PASSWORD`（与 `RU_DEPLOYER_*` 同权）；(b) 明确只支持 `RU_DEPLOYER_HARBOR_PASSWORD` 并修正 `config.toml` / `docs/design.md` 注释。

**影响面**：`src/deploy.rs`（Deployer 结构体 + run_script）、**4 处 `Deployer::new` 调用点**（`multi.rs` / `events.rs` / `global.rs` / `oneshot.rs`，构造参数同步传入 `cfg.harbor`）、`scripts/push_harbor.sh`；release 脚本调用方式不变（位置参数）。
**验收**：`HARBOR_PASSWORD` 为空时推送被跳过；从 `config.toml` 修改 registry 后无需改脚本即生效；`cargo test` 通过。

---

#### T2. DB 历史查询 CLI（建议 P1）

**问题**：部署历史只能查 SQLite 文件，运维排查不便。

**方案**：CLI 增加查询子命令（`clap` 重构为 subcommand 枚举，动作类与轮询类互斥）：
- `ru_deployer history <project> [--branch <b>] [--limit N]` — 复用 `db.recent()` / `recent_by_branch()`
- `ru_deployer stats <project> [--days N]` — 复用 `db.stats()`
- 输出：纯文本表格（列：id、时间、event_type、branch、commit、exit、status、耗时）

**要点**：
- 子命令路径不初始化轮询组件，仅打开 DB；`--config` 仍生效（读 `database.path`）。
- 现有 `--once` 建议一并迁入 subcommand（`once <service>`），保持 CLI 整洁；此为可选重构。
- 需要补 `DeploymentRecord` 的 **`id` 与 `created_at`** 两个字段（当前 `db.rs` 的 `DeploymentRow` 未 select 这两列，`DeploymentRecord` 也没有；输出表格的 id 列与时间列展示需要它们，见 §5 风险 R2）。

**影响面**：`src/main.rs`（CLI 重构）、`src/db.rs`（补 created_at）。
**验收**：`cargo run -- history dev-team/api --limit 5` 输出最近 5 条部署；`stats` 输出总数/成功/失败。

---

#### T3. 回滚功能 — **已决策：不实施**（2026-08-13 业务确认）

**原问题**：部署出错时无法一键回退。

**决策**：**不在本项目实现回滚功能。**

**理由**（业务确认）：
1. 本项目的使用场景是**开发阶段**的自动部署，开发阶段无需回滚；
2. release 在本项目中**仅负责发布镜像**（构建 + 推 Harbor），不负责容器运行态的管理；
3. **回滚的工作不由本项目实现**——由项目之外的工作流承担（如目标机器上按版本拉取 Harbor 镜像重新发布，或发布新的 release）。

**影响**：
- `deployments` 历史表继续留存（审计与排查价值），但不作为回滚数据源使用；
- 原方案中"回滚按 `--deployment-id` 定位"的需求取消，T2 的 `DeploymentRecord` 补充 `id` 字段仅服务于历史展示；
- 原 T3 相关的 git 改动（`ensure_commit`、`fetch_and_reset` 改 `checkout -f`、unshallow/deepen 等）**全部取消**；
- 若将来确实需要代码级回滚，本方案 §3.1 的历史设计要点仍可作参考（已归档于 git 历史与本文档早期版本）。

> 附：原设计要点（归档，不实施）——代码级回滚需新增 `git.rs::ensure_commit`（分支目录内 `reset --hard <commit>`，ref_name 仍传 branch）、回滚记录 `event_type="rollback"`、并发 key 与同分支 push 部署共享；其"临时性"语义（下次 push 自动覆盖）与浅克隆 unshallow 成本是主要限制。

---

#### T4. 日志轮转（建议 P1，低风险快速落地）

**问题**：`/var/log/ru_deployer.log` 无限增长（systemd `append` 模式）。

**方案（推荐 logrotate，零代码改动）**：
- 新增 `/etc/logrotate.d/ru_deployer`：
  ```
  /var/log/ru_deployer.log {
      daily
      rotate 14
      compress
      delaycompress
      copytruncate   # systemd 一直持有 fd，不能用 create
      missingok
      notifempty
  }
  ```
- 说明：`copytruncate` 是 append 模式下唯一安全策略；如需更细粒度可后续改 journald（`StandardOutput=journal` + `journald.conf` 的 `SystemMaxUse`）或引入 `tracing-appender` 滚动文件，两者均为可选演进，本期不做。

**影响面**：仅运维配置（部署文档补充安装步骤），无代码改动。
**验收**：`logrotate -d /etc/logrotate.d/ru_deployer` 调试通过；日志文件按天轮转并压缩。

---

### 3.2 本次 review 新增发现（F1–F7）

#### F1. `ensure_tag()` 的 `--unshallow` 对完整仓库报错（已实证，建议随 T1 修复）

**证据**：本地实验 `git fetch --unshallow` 在完整仓库 exit 128。`ensure_tag()` 的更新路径无条件执行 `--unshallow`：
- 同一 tag 目录第 3 次调用时仓库已非浅（第 2 次 unshallow 后变完整），触发报错 → release 失败。
- 当前触发频率低（`last_release_tag` 内存态防重、重启后 baseline 不触发），但属真实缺陷。

**修复**：执行前用 `git rev-parse --is-shallow-repository` 检测，仅在输出 `true` 时附加 `--unshallow`；否则仅 `git fetch --tags origin`。

#### F2. 文档-实现漂移（随各改动更新文档）

`docs/design.md`、`docs/release-design.md`、`skills/script-writing.md` 仍描述"Rust 注入 `SRC_DIR` / `GITLAB_PROJECT` / `GITLAB_BRANCH` / `GITLAB_COMMIT` / `GITLAB_AUTHOR` / `GITLAB_EVENT_ID`"等环境变量；实际自提交 349918e 起只注入 `GIT_BRANCH` / `VERSION` / `HARBOR_PASSWORD`，脚本自行推导 `SRC_DIR`。**更新三份文档以对齐实现**，避免后续脚本编写者误用。

#### F3. events/global 模式事件积压漏处理（建议 P2）

`events.rs` / `global.rs` 每轮仅处理 `events.first()`（最新一条），两个 poll 间隔内多次 push 时中间事件被跳过；`multi.rs` 已改为遍历全部事件。**统一为遍历全部 + `last_event_id` 过滤**，并注意两点（N7）：
- **提高 per_page**：`events.rs` 用 `get_project_events(&encoded, 5)`（per_page=5），两次 poll 间 >5 次 push 仍会漏；统一提到 10–20；
- **比较语义**：`event_id != last`（`events.rs:49` / `global.rs:49`）应改为 multi 的 `event_id <= last` + `break`，防 GitLab ID 回退/乱序。
- 已知限制：multi 的"遍历全部"也受 `get_project_events_unfiltered` per_page=10 窗口限制（>10 次 push/轮仍漏，可接受，建议注释说明）。

#### F4. 通知收件人二选一逻辑（建议 P2，⚠ 属行为变更）

`multi.rs` / `events.rs` / `global.rs` 中：`repo_emails` 与 `cc_emails` 用 `if … else if`，二者并存时只发 `repo_emails`。**改为合并去重**（`repo_emails + cc`）。⚠ N8：这是**通知量行为变更**而非纯修复——合并后 `notify_author=true` 时 cc 收件人将**每次都收到通知**（当前 cc 仅是 author 缺失时的最后兜底），通知量上升，**需业务确认**后再实施。实现上复用现有 `repo_emails.join(",")` 单收件人通道即可（`notify.rs` 的 `to` 为单元素数组，无需改 API）。

#### F5. release 事件 event_id 恒为 0（建议 P3，先注释说明）

`multi.rs` 中 release 走 Releases API，`ReleaseEvent { event_id: 0, … }`；**event_id 恒为 0**（Releases API 无事件 ID）。影响：
- `get_user_email()` 的 `author_id` **并非恒为 0**：取自 Releases API 响应的 `author.id`（`multi.rs:131`，`types::GitLabRelease.author` 含 `id` 字段），仅当 GitLab 版本未返回 author 对象时才回落为 0（→ 404 → 回落到 repo emails，当前行为可接受）；
- event_id=0 写入 `deployments` 表不会污染 `get_last_event_id()` 的 `MAX(event_id)` 恢复逻辑（正常 push 的 event_id 均 > 0；除非某项目历史上从未有 push 记录）。

**建议**：保持 event_id=0 并加注释说明；author_id 已在用 Releases API 的 author.id，无需改动。**顺带清理（N4）**：`types.rs:158` 的 `ReleaseEvent::from_event` 自 bf8332b 切 Releases API 后成为**死代码**（当前唯一编译警告），实施 F5 时删除该方法（或加 `#[allow(dead_code)]` + 注释保留），使 `cargo build` 重新达到"无警告"。

#### F6. 凭据硬编码（随 T1 一并处理）

`push_harbor.sh` 写死 Harbor 密码、`docker-compose.yml` 与 `dogress.yaml` 写死中间件密码。用户已声明测试环境可接受弱密码，**但写死密码不应进入 git 仓库**：T1 完成后删除脚本内密码；compose/yaml 中的密码建议后续改为 `${VAR:-default}` 环境变量形式（本期可不做，记录在案）。

#### F7. `.gitignore` 未覆盖新项目的拉取目录（建议 P3，随项目新增维护）

`.gitignore` 忽略了 `src/api/`、`src/flint/`、`src/horizon/`、`src/api-manager-platform/`（git 拉取的工作目录），但**新增 dogress 项目后未补 `src/dogress/`**；一旦 multi 模式拉取 dogress 代码，该目录会被 git 跟踪、污染仓库。**建议**：补 `src/dogress/`，并将"新增项目时必须同步 .gitignore"写入 skill（`skills/script-writing.md` 的 checklist）。

---

## 4. 实施顺序与工作量

| 顺序 | 项 | 工作量 | 依赖 | 风险 | 状态 |
|---|---|---|---|---|---|
| 1 | T1 参数统一 + F1 unshallow 修复 + F6 清理 | S（半天） | 无 | 低：需线上回归一次 release | ✅ 完成（fde50a6） |
| 2 | T4 日志轮转 | XS（1 小时，运维） | 无 | 低：copytruncate 会丢轮转瞬间的少量日志，可接受 | ✅ 完成（fde50a6） |
| 3 | T2 历史查询 CLI（含 F2 文档更新） | M（1–2 天） | 无 | 中：CLI 重构影响 `--once`，需回归 | ✅ 完成（73b1850） |
| 4 | F3/F4/F5 小修复 + F2 文档更新 | S–M（1 天） | 无 | 低 | ✅ 完成（d03d766） |

> 全部迭代已于 2026-08-14 完成并提交（fde50a6 / 73b1850 / d03d766）。验收：`cargo build` 无警告（dev + release）、`cargo test` 34/34 通过、history/stats CLI 与 push_harbor.sh 行为均经端到端验证。~~T3 回滚~~ 已确认不实施（§3.1 T3 决策记录）。

---

## 5. 阻塞与风险评估

### 5.1 阻塞问题（Blockers）

**无阻塞问题。** 原 T3 相关的 B1/B2/B3（回滚"临时性"语义、依赖 T2 查询、浅克隆 unshallow 成本）随 T3 **确认不实施**（§3.1）而全部消解，不再构成阻塞。

### 5.2 风险（Risks）

| ID | 项 | 级别 | 说明 |
|---|---|---|---|
| R1 | CLI 重构回归 | 中 | `main.rs` 扁平参数改 subcommand 后，systemd `ExecStart` 与现有 `--mode/--config` 用法需同步验证；`ru_deployer.service` 当前用 `--mode multi --config …`，重构须保持兼容或同步改 unit 文件。 |
| R2 | `DeploymentRecord` 缺 `id` / `created_at` | 低 | 历史查询展示 id/时间列需要补字段；`db.rs` 查询未 select 这两列，需一并补上（`DeploymentRow` + `DeploymentRecord`）。 |
| R3 | T1 线上回归 | 低 | 改 `push_harbor.sh` 后需用一次真实 release 验证推送与跳过两条路径。 |
| R4 | 工作区未提交改动（含本方案文档） | 流程风险 | 当前 master 有大量未提交改动（dogress、oneshot、新 release 脚本），且 `docs/tech-debt-plan.md` 等本方案文档也尚未提交（N9）。**建议实施前先连同方案一起提交打基线**，否则方案与代码状态脱节。 |

### 5.3 结论

- **无阻塞问题**：T1/T2/T4 及 F1–F7 均可按 §4 顺序落地；T3 回滚已确认不实施（§3.1）。
- **建议实施路径**：T1+T4+F1+F6（第一迭代）→ T2+F2+F3+F4+F5（第二迭代）。

---

## 6. 待决策项

1. **CLI 重构范围**：仅加 `history/stats` 子命令，还是同时把 `--once` 迁入 subcommand？
2. **F6 范围**：本期是否同步把 compose/yaml 中的中间件密码改为环境变量形式（默认不做）？
