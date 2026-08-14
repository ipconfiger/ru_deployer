# 测试环境说明

## 服务器

- **测试机**: `172.16.48.71`（root，SSH 密钥免密登录，无需密码）
- 同网段其他机器不可用：`172.16.48.76` 连不上；`172.16.42.36` / `172.16.29.49` / `125.67.215.88` 无 SSH 权限

## 关键路径

| 路径 | 说明 |
|---|---|
| `/opt/gitlab-listener/docker-compose.yml` | 旧版 compose（环境变量基准来源） |
| `/opt/ru_deployer/scripts/docker-compose.yml` | 现用 compose |
| `/opt/ru_deployer/` | ru_deployer 二进制 + config.toml / filter.toml |

## 环境约定

- **测试环境，不要求强密码**：`FLINT_ADMIN_TOKEN=flint-admin-change-me` 等 dev 占位值 / 弱密码可接受，**不要再反复报告弱密码 / 敏感信息暴露类安全问题**。
- compose 操作统一用 `docker compose -p ru_deployer`（项目名为 `ru_deployer`，非目录名 `scripts`）。

## 日志轮转（T4）

`ru_deployer` 以 systemd append 模式写 `/var/log/ru_deployer.log`，无限增长。配置文件位于仓库 `docs/logrotate-ru_deployer.conf`，目标机安装：

```bash
cp docs/logrotate-ru_deployer.conf /etc/logrotate.d/ru_deployer
logrotate -d /etc/logrotate.d/ru_deployer   # 先 dry-run 验证
```

- 必须用 `copytruncate`（systemd 持有 fd，create 无效）；会丢失轮转瞬间的少量日志，可接受。
- 按天轮转，保留 14 份并压缩；`logrotate` 通常由 cron/systemd-timer 每日触发，无需额外配置。
