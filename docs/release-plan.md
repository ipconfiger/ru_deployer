# Release 事件支持 — 实施计划（修订版）

## 阶段总览

```
Phase R1: types + db schema + gitlab 扩展
    ↓
Phase R2: git.rs ensure_tag
    ↓
Phase R3: deploy.rs release() + notify.rs notify_release()
    ↓
Phase R4: multi.rs 事件遍历重写 + 分类处理
    ↓
Phase R5: api_release.sh 脚本
    ↓
Phase R6: 编译、测试、端到端验证
```

## Phase R1: 类型 + DB + API 扩展

- `types.rs`: 新增 `ReleaseEvent` 结构体
  ```rust
  pub struct ReleaseEvent {
      pub project: String,
      pub tag_name: String,
      pub author_name: String,
      pub author_id: u64,
      pub event_id: u64,
  }
  ```
- `db.rs`: `DeploymentRecord` 新增 `event_type: String` 字段
  - SQL migration: `ALTER TABLE ADD COLUMN event_type TEXT NOT NULL DEFAULT 'push'`
  - 新增 `recent_releases()` 查询方法
- `gitlab.rs`: 新增 `get_project_events_unfiltered()` 方法
  - 不带 `action` 参数
  - `per_page=10`（防止无关事件挤压窗口）
  - 共享 `get_project_events()` 的实现，仅 query 参数不同

**验收**: 单元测试，mock API 返回混合事件

## Phase R2: git.rs ensure_tag

- 新增 `ensure_tag(host, token, project, tag) -> PathBuf`
- 目录: `src/<short_name>/<tag>/`
- 首次: `git clone --depth 1 --branch <tag>`
- 更新: `git fetch --tags --unshallow origin && git checkout tags/<tag> && git reset --hard tags/<tag>`

**验收**: 单元测试

## Phase R3: deploy.rs + notify.rs

- `deploy.rs`:
  - 新增 `Deployer::release(event, host, token, cancel_token) -> DeployResult`
  - 调用 `git.ensure_tag()` 拉代码
  - 执行 `scripts/<project>_release.sh`，传入 `VERSION`、`HARBOR_PASSWORD` 等环境变量
  - 返回的 `DeployResult` 中 `event_type = "release"`
- `notify.rs`:
  - 新增 `notify_release(event: &ReleaseEvent, result: &DeployResult, email: &str)`
  - 邮件主题: `[✅/❌] <project> Release <version> 构建<成功/失败>`

**验收**: mock 测试

## Phase R4: multi.rs 事件分类

- 改为遍历所有返回事件（不只看 first）
- 分类逻辑:
  - `push_data.is_some()` → 现有 deploy 流程（需 branch filter）
  - `target_type == "Release"` → 新 release 流程（忽略 branch filter）
- Release 部署 `tokio::spawn` 异步投递，与 push 并行
- 取消机制:
  - push: key = `"api:mysql_db"`
  - release: key = `"api:release:v0.0.70"`（前缀不同，互不冲突）

**验收**: 端到端测试（mock API）

## Phase R5: api_release.sh

- `scripts/api_release.sh`: 编译镜像、调用 `push_harbor.sh` 推 Harbor
- 镜像命名: `gpu_api:<version>`，推送时打 version + latest 双 tag
- 通过 `push_harbor.sh` 处理认证（无密码时优雅跳过）

## Phase R6: 验证

- `cargo test` 全量通过
- 在 GitLab 创建 release，验证完整链路
