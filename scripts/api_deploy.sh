#!/bin/bash
# api_deploy.sh — 部署 dev-team/api
set -e

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)

echo "[api] commit=${COMMIT}"

# 编译 Docker 镜像
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' api_release/Dockerfile
docker build -t api:latest -f api_release/Dockerfile api_release/

# 重启容器
docker compose up -d api

echo "[api] done"
