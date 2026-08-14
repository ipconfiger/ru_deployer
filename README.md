# ru_deployer

一个不折腾的 GitLab 自动部署小工具：盯着 GitLab，有人 push 代码或打 release，就把代码拉下来、跑构建脚本、重启容器，完事发封邮件告诉你结果。

用 Rust 写的，单二进制，装到一台机器上就能跑。不需要 GitLab Runner、不需要公网 IP、不需要 K8s，也不用在 GitLab 里配一堆 webhook。

---

## 为什么会有这个东西

我们团队原来靠一个 Python 脚本轮询 GitLab 接口触发部署，脚本和 shell 里塞了一堆 git 操作、密码、通知逻辑，改一次要小心半天。ru_deployer 是它的 Rust 重写，把能挪进程序里的都挪进去了，剩下的构建逻辑（每个项目不一样，谁也统一不了）留在 shell 脚本里，通过环境变量对接。

如果你也是这种情况——内网 GitLab、几个服务用 docker compose 跑在同一台机器上、push 完想自动构建重启、又不想引入一整套 CI/CD——那这个工具大概率对你有用。

它能干的事：

- **盯着 push 和 release**：轮询 GitLab 接口检测事件，push 触发部署，release（tag）触发构建镜像推 Harbor。不是 webhook，所以没有"webhook 没送达就丢事件"的问题，GitLab 抽风重启了也能靠记录恢复进度。
- **四种模式**：`multi`（默认，多个项目各自独立轮询、并行部署）、`events`（单项目）、`global`（全部可见项目）、`commits`（只看不部署，纯监控）。
- **过滤器**：`filter.toml` 里配哪个项目、哪个分支要管，还能给每个项目配通知兜底邮箱。
- **部署脚本约定**：每个项目一个 `<项目名>_deploy.sh`（构建 + 重启容器）和 `<项目名>_release.sh`（构建 + 推 Harbor）。脚本只负责"这个项目的构建"，git 拉代码、环境变量、超时、取消这些脏活都由程序干。
- **部署历史**：每次部署的结果（成功失败、耗时、输出）记进 SQLite，用 `history` / `stats` 子命令随时查，排查问题不用翻日志。
- **消息通知**：成功/失败发 HTML 邮件，失败带错误日志。收件人先找作者，找不到就发给项目配置的邮箱 + 全局抄送。
- **新 push 顶掉旧部署**：同一个项目同一个分支还在部署时又来了新 push，旧的就地取消（子进程直接杀掉），不会排队堆积。
- **单次手动部署**：`--once --service api` 拉最新代码部署一次就退出，不想等轮询时手动补一发。

## 和常见方案的差别

| 方案 | 优点 | 缺点 | ru_deployer 的情况 |
|---|---|---|---|
| GitLab CI/CD 内置流水线 | 功能全、有 UI、能跑测试矩阵 | 要配 Runner、写 `.gitlab-ci.yml`，小项目杀鸡用牛刀；部署机得能连上 Runner 网络 | 零 Runner、零流水线文件，一个 TOML 配置 + 一个 shell 脚本 |
| webhook 方案（[adnanh/webhook](https://github.com/adnanh/webhook) 这类） | 实时，事件到了立刻触发 | 服务器要能被 GitLab 访问到（公网/内网穿透）；webhook 签名、重试要自己配；**事件丢了没有补偿机制**，GitLab 和服务器任何一方抽风就漏部署 | 轮询天然有补偿：重启后从 SQLite 恢复进度，漏不掉；代价是几秒到十几秒的检测延迟 |
| GitOps 工具（[ArgoCD](https://argo-cd.readthedocs.io/)、[oar](https://github.com/oar-cd/oar)、[composeflux](https://github.com/veerendra2/composeflux) 等） | 声明式、自动收敛，仓库即真相 | 多数面向 K8s；面向 docker compose 的也要引入新组件和新的状态管理概念 | 就是"push 了就去构建重启"，不引入任何抽象，行为直白可预期 |
| [Watchtower](https://github.com/containrrr/watchtower) 这类镜像自动更新 | 简单，只盯着镜像仓库 | 不感知 git push，得先把镜像推上去才生效；构建这一步它不管 | push 事件直接触发构建（`docker build`），构建完再决定要不要推 Harbor |
| Jenkins 等重型 CI | 插件生态、可视化、权限细 | 一套 Java 服务 + 一堆插件要养；配置即代码的坑也不少 | 单二进制 + 一个 systemd 服务 |

一句话：**实时性不如 webhook，功能量不如 CI/CD，但它是"内网单机 docker compose 部署"这个场景里最省心的选择**。轮询延迟默认 10 秒，够用。

## 安装部署

### 1. 编译

需要 Rust 工具链（[rustup.rs](https://rustup.rs) 装一下）：

```bash
cargo build --release
# 产物: target/release/ru_deployer
```

### 2. 部署到目标机

目标机需要：git、docker、docker compose（以及各项目构建依赖，如 node/go）。

```bash
# 假设部署到 /opt/ru_deployer
mkdir -p /opt/ru_deployer
cp target/release/ru_deployer /opt/ru_deployer/
cp config.toml filter.toml /opt/ru_deployer/
cp -r scripts/ /opt/ru_deployer/
```

### 3. 配置

**敏感信息放进 `.env`**（程序启动时自动加载，别写进 config.toml 再提交到 git）：

```bash
cat > /opt/ru_deployer/.env <<'EOF'
RU_DEPLOYER_GITLAB_TOKEN=glpat-xxxxxxxx
HARBOR_PASSWORD=xxxxxxxx
EOF
```

**`config.toml`** 按实际情况改：GitLab 地址、轮询间隔、Harbor 信息、通知平台地址、数据库路径。所有配置项都能用 `RU_DEPLOYER_<段>_<键>` 环境变量覆盖。

**`filter.toml`** 配要监控的项目和分支：

```toml
[[repos]]
project = "dev-team/api"
branches = ["main"]
emails = ["team@example.com"]   # 作者邮箱查不到时兜底通知
```

**部署脚本**：`scripts/` 里给每个项目准备 `<项目名>_deploy.sh`（push 触发）和 `<项目名>_release.sh`（release 触发，可选）。程序会注入 `GIT_BRANCH`（或 `VERSION`）和 `HARBOR_*` 环境变量，脚本自己拼代码目录（`src/<项目>/<分支或tag>/`）——模板见 `scripts/` 下现有脚本，或 `skills/script-writing.md`。

⚠️ 部署到目标机时，把脚本里 `docker compose up -d` 那行的注释去掉（开发机上默认注释掉，避免误重启本机容器）。

### 4. 跑起来

```bash
# 前台试跑
cd /opt/ru_deployer && ./ru_deployer --mode multi

# 用 systemd 托管（服务文件见仓库根目录 ru_deployer.service）
cp ru_deployer.service /etc/systemd/system/
systemctl enable --now ru_deployer

# 日志轮转（避免 /var/log/ru_deployer.log 无限涨）
cp docs/logrotate-ru_deployer.conf /etc/logrotate.d/ru_deployer
logrotate -d /etc/logrotate.d/ru_deployer   # 先 dry-run 验证
```

## 日常使用

```bash
# 查看某项目部署历史
ru_deployer --config /opt/ru_deployer/config.toml history dev-team/api --limit 10
# 只看某分支
ru_deployer --config /opt/ru_deployer/config.toml history dev-team/api --branch main
# 部署统计
ru_deployer --config /opt/ru_deployer/config.toml stats dev-team/api --days 30

# 手动部署一次就退出（不等轮询）
ru_deployer --mode multi --once --service api --branch main

# 纯监控模式（只打印新 commit，不部署）
ru_deployer --mode commits --project dev-team/api --branch main
```

## 项目结构

```
ru_deployer/
├── config.toml            # 主配置（TOML + 环境变量覆盖 + .env）
├── filter.toml            # 项目/分支/通知邮箱过滤器
├── ru_deployer.service    # systemd 单元
├── scripts/               # 各项目部署脚本 + push_harbor.sh + docker-compose.yml
├── src/                   # Rust 源码
│   ├── main.rs            # CLI 入口（含 history/stats 子命令）
│   ├── config.rs / filter.rs / gitlab.rs / git.rs
│   ├── deploy.rs          # 脚本执行 + 取消 + 超时
│   ├── notify.rs          # 邮件通知
│   ├── db.rs              # SQLite 部署历史
│   ├── modes/             # multi / events / global / commits 四种模式
│   └── oneshot.rs         # --once 单次部署
├── skills/                # 新项目接入的脚本编写指南
└── docs/                  # 设计文档、技术债方案、审核报告等
```

## 开发

```bash
cargo test            # 34 个测试：过滤器、配置覆盖、git 集成、CLI、收件人合并等
cargo build           # 应 0 警告
```

设计文档见 `docs/design.md`；技术债与待办见 `docs/tech-debt-plan.md`。

## 已知取舍

- 轮询检测，push 到触发部署有默认 10 秒延迟（`poll_interval_secs` 可调）。
- 单机部署模型，适合"一台机器跑所有容器"的场景；多机/集群请上正经 CI/CD。
- 不做 webhook、不做权限系统、不做回滚——回滚由镜像仓库 + 部署机自行处理，本项目只管"构建和发布"。
