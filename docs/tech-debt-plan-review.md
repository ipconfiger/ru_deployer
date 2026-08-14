# ru_deployer 技术债方案审核报告

**日期**: 2026-08-13
**审核对象**: `docs/tech-debt-plan.md`（技术债与待办治理方案，已吸收 N1–N9 修订）
**审核方式**: 自查（代码逐条核对 + 行为实验）+ 独立 subagent 交叉审核（adversarial review）
**审核结论版本**: 方案文档终版（F1–F7 + N1–N9 全部吸收后）

---

## 1. 结论摘要

**无架构级硬阻塞。** T1/T2/T4 与 F1–F7 均可按方案 §4 顺序落地。

**需求澄清（2026-08-13 业务确认）：T3 回滚功能不在本项目实现** —— 本项目用于开发阶段自动部署，开发阶段无需回滚；release 仅负责发布镜像（构建 + 推 Harbor）；回滚由项目之外的工作流承担。原 T3 相关的 B1/B2/B3 阻塞评估、N1/N2 修正、`ensure_commit`/`checkout -f` 等设计全部随之下架（方案 §3.1 已归档设计要点）。

独立审核确认自查结论基本属实，另发现 **9 项方案与自查均未覆盖的问题（N1–N9）**，其中 3 项在实施前必须处理（已全部吸收回方案文档）：

| ID | 问题 | 级别 |
|---|---|---|
| **N1** | T3 原方案"ref_name 传 commit sha"按字面不可落地（`ensure()` 会 `reset --hard origin/<sha>` 必失败；脚本用 `${GIT_BRANCH}` 拼 `SRC_DIR` 会 cd 到不存在目录） | **方案描述缺陷（已随 T3 下架）** |
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
| T3/B1: 回滚被下次 push 的 reset 覆盖（已随 T3 下架） | 属实 | `git.rs:209-251` `reset --hard origin/<branch>` |
| T3/B3: 浅克隆需 unshallow；unshallow 后会被再次标记为浅（已随 T3 下架） | 属实（措辞已修） | `git.rs:192` `--depth 1`；实测 N6 |
| F1: ensure_tag 更新路径无条件 `--unshallow` | 属实 | `git.rs:125-130` |
| F3: events/global 只处理 first()，events per_page=5 | 属实 | `events.rs:44/42`、`global.rs:44` |
| F4: 收件人 if/else if 二选一 | 属实 | multi:319-325 / events:120-124 / global:126-130 |
| F5: event_id 恒 0；author_id 非恒 0 | 属实（已修正） | `multi.rs:127-133`；`types.rs:177-187` |
| T4: append 模式需 copytruncate | 属实 | `ru_deployer.service:14-15` |

---

## 4. 审核修正记录（全部已同步回方案文档）

自查修正（6 处）：
1. T1 影响面补 4 处 `Deployer::new` 调用点；
2. T3 并发 key 必须与同分支 push 部署共享（独立 `:rollback` key 会并行踩踏同一 git 目录）——（随 T3 下架）；
3. F5 论断：author_id 取自 Releases API author.id，非恒 0；
4. B3 措辞：删除"每次 fetch 开销上升"；
5. T1 补充：config.rs 需补 `RU_DEPLOYER_HARBOR_REGISTRY/PROJECT/USER` env override；
6. T2 补充：`DeploymentRecord` 需补 `id` + `created_at`。

独立审核修正（N1–N9，已全部吸收；N1/N2 随 T3 下架而归档）：
- **N1**（T3 第 3 步，已归档）：ref_name 必须仍传 branch，`ensure_commit` 在分支目录内 `reset --hard <commit>` 完成切换；
- **N2**（T3 新增风险，已归档）：脚本 `sed -i` 弄脏已跟踪文件 → 必须用 `reset --hard` 而非 `checkout`；回滚后 detached HEAD + 脏文件会干扰下次 push 的 `checkout <branch>` → `fetch_and_reset` 改 `checkout -f`；
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

### 5.2 T3 回滚：已决策不实施（2026-08-13 业务确认）

| 维度 | 分析 |
|---|---|
| 决策 | **不在本项目实现回滚**：开发阶段无需回滚；release 仅发布镜像（推 Harbor）；回滚由项目之外的工作流承担 |
| 原评估 | 原技术可行性（代码级回滚）与语义限制（"临时回滚"、unshallow 成本）分析仍有效，但**不再适用**——随需求澄清全部下架 |
| 遗留影响 | `deployments` 历史表继续留存（审计/排查），不作为回滚数据源；原 N1/N2 修正与 `ensure_commit` 设计归档于方案 §3.1 |

### 5.3 实施前必办清单（源自 N3/N4）

1. T1 实施时：处理 N3（config.rs 补读 plain `HARBOR_PASSWORD` 或改文档注释）；
2. 第一迭代前：删除 `ReleaseEvent::from_event` 死代码（N4），恢复"无警告"；
3. 先提交当前工作区改动 + 本方案文档打基线（R4/N9，已完成：4 个提交 0c19fce…f5a9d5d）。

---

## 6. 独立审核交叉验证结论

独立 subagent 对方案逐条对照源码并复核了全部行为实验，确认：
- 12 项论断核对（a–h）全部属实或已修正，无实质性分歧；
- 方案默认实施顺序（T1+T4+F1+F6 → T2+F2+F3+F4+F5；原 "→ T3 单独评审" 已因需求澄清取消）合理，无优先级异议；
- 发现的 N1–N9 均已吸收进方案文档，其中 N3（T1 环境变量回归）、N4（无警告论断不实）为实施前必须处理项；N1（T3 描述缺陷）已随 T3 下架。

---

## 7. 结论

- **无阻塞问题**：T1/T2/T4 与 F1–F7 均可实施；方案经两轮审核（自查 + 独立 adversarial review）修正 15 处后，与代码事实一致。
- **T3 回滚已确认不实施**（业务决策，2026-08-13），原阻塞评估随之下架。
- 建议：按 §4 顺序执行第一迭代（T1+T4+F1+F6，含 N3/N4），再第二迭代（T2+F2+F3+F4+F5）。基线已提交（0c19fce…f5a9d5d）。
