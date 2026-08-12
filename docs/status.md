# ru_deployer 开发状态

**日期**: 2026-08-12
**分支**: master (`bf8332b`)
**远程**: https://github.com/ipconfiger/ru_deployer.git

## 已实现功能

### 核心功能 ✅

| 功能 | 状态 | 说明 |
|---|---|---|
| Push 事件检测 | ✅ | 通过 GitLab Events API 轮询，4 个项目独立监控 |
| Release 事件检测 | ✅ | 通过 GitLab Releases API 轮询，按 tag 检测新 release |
| 四种轮询模式 | ✅ | multi(默认), events, global, commits(仅监控) |
| 项目/分支过滤器 | ✅ | filter.toml，支持 per-repo 分支和 emails 配置 |
| Git 代码拉取 | ✅ | `src/<project>/<branch>/`，支持 branch 和 tag |
| 部署脚本执行 | ✅ | `<project>_deploy.sh`，docker build + compose up |
| Release 构建 | ✅ | `<project>_release.sh`，带版本号 + Harbor 推送 |
| 邮件通知 | ✅ | 成功/失败 HTML 邮件，优先 GitLab email → repo emails → cc |
| SQLite 部署历史 | ✅ | 记录每次 push/release，含 stdout/stderr 截断 |
| 并发控制 | ✅ | 同 project:branch 取消旧任务，不同 key 完全并行 |
| 信号处理 | ✅ | SIGTERM/SIGINT 优雅退出 |
| systemd 服务 | ✅ | `ru_deployer.service` |
| 第一轮基线 | ✅ | 首次 poll 不处理事件，只建 baseline |

### 质量 ✅

| 指标 | 状态 |
|---|---|
| 编译警告 | 0 |
| 单元测试 | 19/19 pass |
| Oracle 审核 | 2 轮通过（设计 + 代码） |

## 目录结构

```
ru_deployer/
├── Cargo.toml / Cargo.lock
├── config.toml              # 主配置
├── filter.toml              # 项目/分支/emails 过滤器
├── ru_deployer.service      # systemd unit
├── .gitignore
├── data/                    # SQLite DB (运行时)
├── scripts/                 # 部署脚本（新建，不修改 tools/ 原文件）
│   ├── api_deploy.sh
│   ├── api_release.sh
│   ├── flint_deploy.sh
│   ├── horizon_deploy.sh
│   ├── api-manager-platform_deploy.sh
│   ├── push_harbor.sh
│   └── docker-compose.yml
├── src/                     # Git 拉取的工作目录 (运行时)
├── src/                     # Rust 源码
│   ├── main.rs              # CLI 入口
│   ├── config.rs            # TOML + 环境变量覆盖
│   ├── types.rs             # 共享类型
│   ├── filter.rs            # 过滤器
│   ├── gitlab.rs            # GitLab API 客户端
│   ├── git.rs               # git clone/fetch/ensure_tag
│   ├── deploy.rs            # 脚本执行 + CancellationToken 取消
│   ├── notify.rs            # 邮件通知 (notify_release 双接口)
│   ├── db.rs                # SQLite 部署历史
│   ├── logging.rs           # tracing 日志
│   └── modes/
│       ├── multi.rs         # 多项目独立轮询 (默认)
│       ├── events.rs        # 单项目 events
│       ├── global.rs        # 全局 events
│       └── commits.rs       # 仅监控
└── docs/
    ├── design.md            # 完整设计文档
    ├── dev-plan.md          # 开发计划
    ├── release-design.md    # Release 功能设计
    ├── release-plan.md      # Release 实施计划
    └── status.md            # 本文件
```

## 部署说明

### 开发机运行

```bash
cd /home/alex/Projects/ru_deployer
export RU_DEPLOYER_GITLAB_TOKEN="glpat-xxx"
cargo run -- --mode multi
```

注意：`scripts/*.sh` 中 `docker compose up -d` 已注释，部署到测试机时取消注释。

### 生产部署

```bash
# 编译
cargo build --release

# 安装
cp target/release/ru_deployer /opt/ru_deployer/
cp config.toml filter.toml /opt/ru_deployer/
cp -r scripts/ /opt/ru_deployer/
cp ru_deployer.service /etc/systemd/system/

# 设置 token
echo 'RU_DEPLOYER_GITLAB_TOKEN=glpat-xxx' > /opt/ru_deployer/.env

# 取消 docker compose 注释
sed -i 's/^# docker compose/docker compose/' /opt/ru_deployer/scripts/*.sh

# 启动
systemctl enable --now ru_deployer
```

### 注意事项

- 部署前取消 `scripts/*.sh` 中 `docker compose up -d` 的注释
- `config.toml` 中的 `token` 可留空，通过 `.env` 或环境变量注入
- filter.toml 中每个 repo 可配置 `emails` 作为通知 fallback
- DB 文件 `data/deploy.db` 需可写，自动创建

## 技术债务 / 待办

| 项目 | 优先级 | 说明 |
|---|---|---|
| `push_harbor.sh` 参数统一 | 低 | release 脚本用 HARBOR_PASSWORD，deploy 脚本用 push_harbor.sh |
| DB 历史查询 CLI | 低 | 添加 `--show-recent` 等子命令查看部署历史 |
| 回滚功能 | 低 | 基于 SQLite 历史实现回滚 |
| 日志轮转 | 低 | 当前依赖 systemd journal |

## Git 提交历史

```
bf8332b fix: Release 检测改用 Releases API 轮询
1a7745c fix: Oracle 审核 B1/I1/I2 — 第一轮基线 + VERSION + DB 字段
4537f00 fix: event_id 硬编码 0 导致每次启动重跑所有历史事件
af1db2f fix: 每次启动重复部署 — last_event_id 从 DB 恢复
12c9f71 fix: 屏蔽本机 docker compose + 事件风暴修复 + DB 错误详情日志
b061736 feat: Release 事件支持 v1.0
39e78f5 refactor: 方案B并发控制 + 清零所有warning
3ca15c2 feat: ru_deployer v0.1.0 — Rust rewrite of listen_push.py
```
