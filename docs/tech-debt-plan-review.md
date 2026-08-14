# ru_deployer 技术债方案审核报告

**日期**: 2026-08-13
**审核对象**: `docs/tech-debt-plan.md`（技术债与待办治理方案，已吸收 N1–N9 修订）
**审核方式**: 自查（代码逐条核对 + 行为实验）+ 独立 subagent 交叉审核（adversarial review）
**审核结论版本**: 方案文档终版（F1–F7 + N1–N9 全部吸收后）

---

## 1. 结论摘要

**无架构级硬阻塞。** T1/T2/T4 与 F1–F7 均可按方案 §4 顺序落地；唯一需要决策的是 T3 回滚功能的语义定位（"临时回滚"可接受则可行，否则暂缓重设计）。

独立审核确认自查结论基本属实，另发现 **9 项方案与自查均未覆盖的问题（N1–N9）**，其中 3 项必须在实施前修正（已全部吸收回方案文档）：

| ID | 问题 | 级别 |
|---|---|---|
| **N1** | T3 原方案"ref_name 传 commit sha"按字面不可落地（`ensure()` 会 `reset --hard origin/<sha>` 必失败；脚本用 `${GIT_BRANCH}` 拼 `SRC_DIR` 会 cd 到不存在目录） | **方案描述缺陷（已修正）** |
| **N3** | T1 删除密码兜底后，plain `HARBOR_PASSWORD`（config.toml/design.md 声称可用）将静默失效，线上 release 静默跳过推送 | 中（已修正） |
| **N4** | 方案 §2.1"cargo build 无警告"不属实：`ReleaseEvent::from_event`（types.rs:158）为死代码，当前有 1 个警告 | 低（已修正） |

---

## 2. 行为实验与核对（自查 + 独立审核交叉验证）

| 实验/核对 | 结果 | 验证方 |
|---|---|---|
| `cargo build` | 通过；**1 个 dead_code 警告**（`ReleaseEvent::from_event`） | 双方 |
| `cargo test` | 25/25 通过 | 双方 |
| `git fetch --unshallow`（完整仓库） | **exit 128 报错** → 证实 F1 | 双方 |
| `git fetch --unshallow`（浅仓库 file:// 协议） | exit 0；`is-shallow-repository` true→false | 双方 |
| `git fetch --depth 1`（完整仓库） | exit 0，但 **`is-shallow-repository` 变回 true、旧 commit 重新不可达**（HEAD~3 不可解析）→ 修正 B3 措辞（N6） | 双方（独立审核首发） |
| `Deployer::new` 调用点 grep | 4 处（multi/events/global/oneshot） | 双方 |
| `multi.rs:131` release author_id | 取自 Releases API `author.id`，非恒 0 | 双方 |
| `.gitignore` 覆盖范围 | 未含 `src/dogress/` → F7 | 双方 |
| `get_user_email` 404 行为 | 返回 `Ok(String::new())` 非报错 → 回落 repo emails 正确 | 独立审核 |
| deploy 脚本 `SRC_DIR` 推导 | 全部用 `${ROOT_DIR}/src/${PROJECT}/${GIT_BRANCH}` 拼目录 | 独立审核 |

---

## 3. 论断核对表（双方一致结论）

| 方案论断 | 判定 | 代码证据 |
|---|---|---|
| T1: run_script 仅注入 3 个环境变量 | 属实 | `deploy.rs:242-244` |
| T1: push_harbor.sh 密码兜底使"跳过"永不生效 | 属实 | `push_harbor.sh:21` `${HARBOR_PASSWORD:-dymkCr…}` |
| T1: Deployer 仅持 password，构造点 4 处 | 属实 | `deploy.rs:49-69`；multi:59 / events:23 / global:23 / oneshot:61 |
| T2: db.rs 三个查询已实现且 dead_code | 属实 | `db.rs:164/186/214` |
| T2/R2: DeploymentRecord 缺 created_at（且缺 id） | 属实 | `db.rs:17-30`；`DeploymentRow` L250 未 select |
| T3/B1: 回滚被下次 push 的 reset 覆盖 | 属实 | `git.rs:209-251` `reset --hard origin/<branch>` |
| T3/B3: 浅克隆需 unshallow；unshallow 后会被再次标记为浅 | 属实（措辞已修） | `git.rs:192` `--depth 1`；实测 N6 |
| F1: ensure_tag 更新路径无条件 `--unshallow` | 属实 | `git.rs:125-130` |
| F3: events/global 只处理 first()，events per_page=5 | 属实 | `events.rs:44/42`、`global.rs:44` |
| F4: 收件人 if/else if 二选一 | 属实 | multi:319-325 / events:120-124 / global:126-130 |
| F5: event_id 恒 0；author_id 非恒 0 | 属实（已修正） | `multi.rs:127-133`；`types.rs:177-187` |
| T4: append 模式需 copytruncate | 属实 | `ru_deployer.service:14-15` |

---

## 4. 审核修正记录（全部已同步回方案文档）

自查修正（6 处）：
1. T1 影响面补 4 处 `Deployer::new` 调用点；
2. T3 并发 key 必须与同分支 push 部署共享（独立 `:rollback` key 会并行踩踏同一 git 目录）；
3. F5 论断：author_id 取自 Releases API author.id，非恒 0；
4. B3 措辞：删除"每次 fetch 开销上升"；
5. T1 补充：config.rs 需补 `RU_DEPLOYER_HARBOR_REGISTRY/PROJECT/USER` env override；
6. T2 补充：`DeploymentRecord` 需补 `id` + `created_at`。

独立审核修正（N1–N9，已全部吸收）：
- **N1**（T3 第 3 步）：ref_name 必须仍传 branch，`ensure_commit` 在分支目录内 `reset --hard <commit>` 完成切换；
- **N2**（T3 新增风险）：脚本 `sed -i` 弄脏已跟踪文件 → 必须用 `reset --hard` 而非 `checkout`；回滚后 detached HEAD + 脏文件会干扰下次 push 的 `checkout <branch>` → `fetch_and_reset` 改 `checkout -f`；
- **N3**（T1 新增风险）：plain `HARBOR_PASSWORD` 静默失效 → config.rs 补读或改文档，二选一；
- **N4**（§2.1 事实修正 + F5 补充）：`ReleaseEvent::from_event` 死代码，实施时删除；
- **N5**：T2/R2 需补 `id`（与自查 #6 合并）；
- **N6**（B3 措辞）：`fetch --depth 1` 会把完整仓库重新标记为浅；
- **N7**（F3 补充）：events.rs per_page 5→10–20；`!=` 改 `<=`+break；
- **N8**（F4 标注）：合并收件人是通知量行为变更，需业务确认；
- **N9**（R4 强化）：方案文档本身未纳入 git 基线。

---

## 5. 阻塞问题判定

### 5.1 无硬阻塞

| 项 | 判定 | 依据 |
|---|---|---|
| T1 参数统一 | ✅ 可实施 | 改动面明确（deploy.rs + 4 构造点 + push_harbor.sh）；N3 处理后可安全落地 |
| T2 历史查询 CLI | ✅ 可实施 | db.rs 查询方法现成；补 id/created_at + CLI 重构（注意 systemd 兼容，R1） |
| T4 日志轮转 | ✅ 可实施 | logrotate copytruncate 零代码改动 |
| F1–F7 | ✅ 可实施 | 均为局部修改，F1 为唯一已实证线上缺陷（修复方案已定） |

### 5.2 唯一需决策项：T3 回滚

| 维度 | 分析 |
|---|---|
| 技术可行性 | 代码级回滚可行（`ensure_commit` 在分支目录内 `reset --hard` + ref_name 仍传 branch + 共享并发 key + `checkout -f` 连带修复）。**N1 修正后方案第 3 步不再自相矛盾** |
| 语义阻塞 | 回滚是**临时**的：下次同分支 push 部署自动覆盖。若团队预期"回滚后保持旧版本"，需冻结标志/分支锁（架构级改动），属超出技术债范围的重新设计 |
| 成本 | unshallow 首次拉全历史（大仓库耗时/磁盘不可控）；且 N6 证实"回滚→正常部署"后仓库再次变浅，再次回滚仍需 unshallow。缓解：`--deepen N` 或**仅支持"回滚到最近一次成功部署的 commit"（推荐默认）** |
| 建议 | **默认接受"临时回滚"语义、限定最近一次成功 commit 后按 P2 实施；若不可接受，暂缓 T3 并单独立项** |

### 5.3 实施前必办清单（源自 N1/N3/N4）

1. T3 若立项：按 N1/N2 修正落地（方案已写清）；
2. T1 实施时：处理 N3（config.rs 补读 plain `HARBOR_PASSWORD` 或改文档注释）；
3. 第一迭代前：删除 `ReleaseEvent::from_event` 死代码（N4），恢复"无警告"；
4. 先提交当前工作区改动 + 本方案文档打基线（R4/N9）。

---

## 6. 独立审核交叉验证结论

独立 subagent 对方案逐条对照源码并复核了全部行为实验，确认：
- 12 项论断核对（a–h）全部属实或已修正，无实质性分歧；
- 方案默认实施顺序（T1+T4+F1+F6 → T2+F2+F3+F4+F5 → T3 单独评审）合理，无优先级异议；
- 发现的 N1–N9 均已吸收进方案文档，其中 N1（T3 描述缺陷）、N3（T1 环境变量回归）、N4（无警告论断不实）为实施前必须处理项。

---

## 7. 结论

- **无阻塞问题**：T1/T2/T4 与 F1–F7 均可实施；方案经两轮审核（自查 + 独立 adversarial review）修正 15 处后，与代码事实一致。
- 唯一决策点：**T3 回滚语义**（"临时回滚"可行 vs 暂缓重设计），需业务拍板；若实施，按"最近一次成功部署的 commit"为默认入口。
- 建议：先提交工作区 + 方案打基线，然后按 §4 顺序执行第一迭代（T1+T4+F1+F6，含 N3/N4）。
