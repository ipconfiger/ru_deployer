#!/bin/bash
# horizon_deploy.sh — 部署 dev-team/horizon (Node/Nitro 前端)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="horizon"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${GIT_BRANCH}"

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)
echo "[horizon] branch=${GIT_BRANCH} commit=${COMMIT}"

# 编译 Docker 镜像
docker build -t horizon:latest -f Dockerfile \
    --build-arg VITE_PRODUCT="gpu" \
    --build-arg VITE_HORIZON_API_ENDPOINT="http://api:3000" \
    --build-arg VITE_HORIZON_MANAGER_API_ENDPOINT="http://api-manager-platform:8080" \
    --build-arg VITE_WEBRIFY_TURNSTILE_ENDPOINT="https://webrify.do.top" \
    --build-arg VITE_HORIZON_CMS_API_ENDPOINT="http://172.16.42.32:8080" \
    --build-arg VITE_API_ENDPOINT="https://api.do.top/v1" \
    --build-arg VITE_PLAYGROUND_API_ENDPOINT="http://172.16.42.36:3001/v1" \
    .

# 重启容器
docker compose up -d horizon

echo "[horizon] done"
