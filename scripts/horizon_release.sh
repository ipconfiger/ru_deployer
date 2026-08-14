#!/bin/bash
# horizon_release.sh — Release 构建 dev-team/horizon (Node/Nitro 前端)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="horizon"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${VERSION}"

cd "${SRC_DIR}"
echo "[horizon_release] version=${VERSION}"

# 编译 Docker 镜像
docker build -t "gpu_horizon:${VERSION}" -t "gpu_horizon:latest" -f Dockerfile \
    --build-arg VITE_PRODUCT="gpu" \
    --build-arg VITE_HORIZON_API_ENDPOINT="http://api:3000" \
    --build-arg VITE_HORIZON_MANAGER_API_ENDPOINT="http://api-manager-platform:8080" \
    --build-arg VITE_WEBRIFY_TURNSTILE_ENDPOINT="https://webrify.do.top" \
    --build-arg VITE_HORIZON_CMS_API_ENDPOINT="http://172.16.42.32:8080" \
    --build-arg VITE_API_ENDPOINT="https://api.do.top/v1" \
    --build-arg VITE_PLAYGROUND_API_ENDPOINT="http://172.16.42.36:3001/v1" \
    --build-arg VITE_APP_VERSION="${VERSION}" \
    .

# 推送到 Harbor
"${SCRIPT_DIR}/push_harbor.sh" "gpu_horizon:${VERSION}" "gpu_horizon" "${VERSION}"

echo "[horizon_release] done"
