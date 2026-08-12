# Release 事件支持 — 设计文档

## 1. 需求

检测 GitLab Release 事件，触发对应的 release 构建流程。

### 1.1 事件检测

- 在 `multi` 模式下，轮询项目 events 时**额外查询无 `action` 过滤的事件**
- 检测 `target_type == "Release"` 的事件
- Release 事件**忽略 filter 的分支限制**（任何分支的 release 都触发）
- 仅 filter 中配置的项目才触发

### 1.2 版本号获取

- 从 release 事件的 `target_title` 获取 tag 名作为版本号（如 `v0.0.70`）

### 1.3 代码拉取

- 按 tag 拉取代码到 `src/<short_name>/<tag_name>/`（如 `src/api/v0.0.70/`）
- 使用 `git clone --depth 1 --branch <tag> <url> <target>` 或 `git fetch --tags` + `git checkout <tag>`

### 1.4 脚本执行

- 执行 `scripts/<short_name>_release.sh`（如 `api_release.sh`）
- 传入环境变量：
  - `SRC_DIR`: 代码目录（绝对路径）
  - `VERSION`: 版本号（tag 名）
  - `GITLAB_PROJECT`, `GITLAB_BRANCH` 等（同上）

### 1.5 Release 脚本与 Deploy 脚本的区别

| 项目 | `_deploy.sh` | `_release.sh` |
|---|---|---|
| 触发条件 | push 事件 | release 事件 |
| 镜像名 | `<project>:latest` | `gpu_<project>:<version>` |
| 版本号 | git commit short SHA | release tag 名 |
| Harbor 推送 | 不推 | 推送到 Harbor（带版本 tag + latest） |
| Docker compose up | ✅ | 可选的 |

---

## 2. 实现方案

### 2.1 新增类型

```rust
// types.rs 新增
pub struct ReleaseEvent {
    pub project: String,     // "dev-team/api"
    pub tag_name: String,    // "v0.0.70"  (版本号)
    pub author_name: String,
    pub author_id: u64,
    pub event_id: u64,
}
```

### 2.2 Git API 扩展

```rust
// gitlab.rs 新增方法
impl GitLabClient {
    /// 获取项目的 release 列表（最新 N 条）
    pub async fn get_releases(&self, project_path: &str, per_page: u32) -> Result<Vec<Release>>;
}
```

但 release 事件的检测**不依赖 Releases API**。GitLab Events API 在创建 release 时会生成一个事件：
- `action_name`: 留空或特定值
- `target_type`: `"Release"`
- `target_title`: tag 名称

所以只需在现有事件轮询中去掉 `action=pushed` 过滤，就能同时拿到 push 和 release 事件。

### 2.3 multi.rs 改动 — 事件遍历逻辑重写（I-R3 修复）

**核心问题**：去掉 `action` 参数后，`events.first()` 可能是 issue/comment 等无关事件，导致 push/release 被跳过。

**修复**：遍历返回的**全部事件**，逐个处理，而不是只看第一条：

```rust
// 伪代码
let events = client.get_project_events_unfiltered(&encoded_path, 10).await;
let new_last_id = events.first().map(|e| e.id);  // 最新事件 ID 作为下次比较基准

for event in &events {
    // 跳过已处理的事件
    if event.id <= last_id {
        break;
    }

    if event.push_data.is_some() {
        // push 事件 — 现有逻辑，需匹配 branch filter
        let branch = event.push_data.as_ref().unwrap().r#ref.replace("refs/heads/", "");
        if filter.matches(project, branch) {
            let push_event = PushEvent::from_event(event, Some(project));
            spawn_deploy(push_event, "push");
        }
    } else if event.target_type.as_deref() == Some("Release") {
        // release 事件 — 忽略 branch filter
        let tag_name = match event.target_title.as_deref() {
            Some(t) => t,
            None => {
                warn!("Release event {} has no target_title, skipping", event.id);
                continue;
            }
        };
        let release_event = ReleaseEvent {
            project: project.clone(),
            tag_name: tag_name.to_string(),
            author_name: event.author.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
            author_id: event.author_id,
            event_id: event.id,
        };
        spawn_release(release_event);
    }
}

state.last_event_id = new_last_id;  // 注意：events 为空时 new_last_id 为 None，不应覆盖
// 正确写法：if let Some(id) = new_last_id { state.last_event_id = Some(id); }
```

> 关键与当前 push-only 逻辑的区别：**遍历全部事件**而非只看 `first()`，每种事件类型分别处理，`last_event_id` 更新为返回列表中最新的 ID（不论类型）。

**ActiveDeploy key 区分**：
- push 部署: `"project:branch"` → `"api:mysql_db"`
- release 部署: `"project:release:tag"` → `"api:release:v0.0.70"`
- 两者共用同一个 `DashMap`，key 前缀不同避免冲突

### 2.4 git.rs 改动 — ensure_tag

```rust
impl GitRepo {
    /// 按 tag 拉取代码（release 专用）
    pub async fn ensure_tag(
        &self, gitlab_host: &str, token: &str,
        project: &str, tag: &str,
    ) -> Result<PathBuf>;
}
```

实现：
```
ensure_tag(project="dev-team/api", tag="v0.0.70")
  → target_dir = src/api/v0.0.70/
  → if .git 存在:
      git fetch --tags --unshallow origin  // 解决 shallow clone 取不到旧 tag 的问题
      git checkout tags/v0.0.70
      git reset --hard tags/v0.0.70
  → else:
      git clone --depth 1 --branch v0.0.70 <url> <target>
```

> `--unshallow` 将 shallow clone 转换为完整仓库，确保任意 commit 的 tag 都可达。如 tag 指向的 commit 已在 shallow 范围内则无额外开销；否则自动 deepen。

### 2.5 deploy.rs 改动 — release() 方法

```rust
impl Deployer {
    pub async fn release(
        &self, event: &ReleaseEvent,
        gitlab_host: &str, gitlab_token: &str,
        cancel_token: CancellationToken,
    ) -> DeployResult;
}
```

与 `deploy()` 的区别：
- 调用 `git.ensure_tag()` 而非 `git.ensure()`
- 执行 `<project>_release.sh` 而非 `<project>_deploy.sh`
- 额外传入 `VERSION` 环境变量（**注意：`HARBOR_PASSWORD` 同样需要传入**，`Deployer` 结构体已持有该字段，直接复用）
- `event_type` 由 multi.rs 在创建 `DeploymentRecord` 时设置为 `"release"`（不在 `DeployResult` 中传递）

### 2.6 notify.rs 改动 — 新增 notify_release

```rust
impl Notifier {
    /// 发送 release 构建结果通知
    pub async fn notify_release(
        &self, event: &ReleaseEvent, result: &DeployResult, author_email: &str
    );
}
```

邮件主题：`[✅/❌] <project> Release <version> 构建<成功/失败>`

内容与 push 通知结构相同，但 `branch` 替换为合 `tag_name`（即版本号）。

### 2.7 db.rs 改动 — event_type 字段

`DeploymentRecord` 新增字段：

```rust
pub event_type: String,  // "push" | "release"
```

SQL schema 新增：

```sql
ALTER TABLE deployments ADD COLUMN event_type TEXT NOT NULL DEFAULT 'push';
```

已有记录默认 `'push'`（向后兼容）。release 构建时传入 `"release"`。

新增查询方法：

```rust
pub async fn recent_releases(&self, project: &str, limit: u32) -> Result<Vec<DeploymentRecord>>;
// WHERE project = ? AND event_type = 'release' ORDER BY id DESC LIMIT ?
```

---

## 3. 需要新增/修改的文件

| 文件 | 改动 | 说明 |
|---|---|---|
| `src/types.rs` | 新增 `ReleaseEvent` 类型 | 独立于 PushEvent |
| `src/gitlab.rs` | 新增 `get_project_events_unfiltered()` 方法 | 不带 action 参数，per_page=10 |
| `src/git.rs` | 新增 `ensure_tag()` 方法 | 按 tag checkout，用 `--unshallow` 解决 shallow clone 问题 |
| `src/deploy.rs` | 新增 `release()` 方法 | 传 VERSION、HARBOR_PASSWORD，执行 `_release.sh` |
| `src/db.rs` | `DeploymentRecord` 新增 `event_type` 字段 | "push" / "release"，SQL schema 加列 |
| `src/notify.rs` | 新增 `notify_release()` 方法 | 邮件主题含版本号 |
| `src/modes/multi.rs` | **事件遍历逻辑重写**: 遍历全部事件 + 分类处理 | ⬇ 见 §2.3 |
| `scripts/api_release.sh` | 新增 | 调用 push_harbor.sh，与 deploy 风格一致 |

### 3.1 不需要改的

- `filter.rs` — release 忽略 branch filter，但 project 匹配逻辑复用
- `events.rs`, `global.rs`, `commits.rs` — release 仅 multi 模式支持，在设计开头已声明

---

## 4. Release 脚本模板（`api_release.sh`）

与 deploy 脚本风格一致，调用 `push_harbor.sh`，通过环境变量传入认证信息。

```bash
#!/bin/bash
# api_release.sh — Release 构建 dev-team/api
# 环境变量: SRC_DIR, VERSION, HARBOR_PASSWORD, SCRIPTS_DIR
set -e

SCRIPT_DIR="${SCRIPTS_DIR:-$(cd "$(dirname "$0")" && pwd)}"

cd "${SRC_DIR}"
echo "[api_release] version=${VERSION}"

# 编译镜像
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' api_release/Dockerfile
docker build -t "gpu_api:${VERSION}" -f api_release/Dockerfile api_release/

# 推送到 Harbor（复用 push_harbor.sh，优雅跳过无密码场景）
"${SCRIPT_DIR}/push_harbor.sh" "gpu_api:${VERSION}" "gpu_api" "${VERSION}"

echo "[api_release] done"
```

> 其他项目如需 release，按相同模式创建 `<project>_release.sh`，修改 Dockerfile 路径和镜像名前缀即可。
