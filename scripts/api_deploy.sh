#!/bin/bash
# api_deploy.sh — 部署 dev-team/api
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="api"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${GIT_BRANCH}"

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)
echo "[api] branch=${GIT_BRANCH} commit=${COMMIT}"

# 编译 Docker 镜像
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' api_release/Dockerfile
docker build -t api:latest -f api_release/Dockerfile api_release/

# 重启容器
docker compose up -d api

echo "[api] done"
