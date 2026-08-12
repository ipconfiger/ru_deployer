#!/bin/bash
# horizon_deploy.sh — 部署 dev-team/horizon (Node/Nitro 前端)
# Rust 传入的环境变量:
#   GITLAB_PROJECT   e.g. "dev-team/horizon"
#   GITLAB_BRANCH    e.g. "feat/multi"
#   GITLAB_COMMIT    完整 SHA
#   GITLAB_AUTHOR    提交者用户名
#   SRC_DIR          Rust 已拉好的代码目录
#   SCRIPTS_DIR      脚本所在目录
#   HARBOR_PASSWORD  Harbor 密码
set -e

SCRIPT_DIR="${SCRIPTS_DIR:-$(cd "$(dirname "$0")" && pwd)}"
PROJECT_NAME="${GITLAB_PROJECT##*/}"

echo "=========================================="
echo "[${PROJECT_NAME}_deploy] 开始部署 ${GITLAB_PROJECT}"
echo "  分支:       ${GITLAB_BRANCH}"
echo "  commit:     ${GITLAB_COMMIT}"
echo "  作者:       ${GITLAB_AUTHOR}"
echo "=========================================="
echo ""

cd "${SRC_DIR}"
ACTUAL_COMMIT=$(git rev-parse --short HEAD)
echo "[${PROJECT_NAME}] 当前 commit: ${ACTUAL_COMMIT}"

# 编译 Docker 镜像（Nitro 前端需要 --build-arg）
IMAGE_NAME="${PROJECT_NAME}:latest"
DOCKERFILE="${SRC_DIR}/Dockerfile"

if [ ! -f "${DOCKERFILE}" ]; then
    echo "错误: 未找到 Dockerfile: ${DOCKERFILE}" >&2
    exit 1
fi

echo "[${PROJECT_NAME}] 开始编译 Docker 镜像: ${IMAGE_NAME}"
docker build -t "${IMAGE_NAME}" -f "${DOCKERFILE}" \
    --build-arg VITE_PRODUCT="gpu" \
    --build-arg VITE_HORIZON_API_ENDPOINT="http://api:3000" \
    --build-arg VITE_HORIZON_MANAGER_API_ENDPOINT="http://api-manager-platform:8080" \
    --build-arg VITE_WEBRIFY_TURNSTILE_ENDPOINT="https://webrify.do.top" \
    --build-arg VITE_HORIZON_CMS_API_ENDPOINT="http://172.16.42.32:8080" \
    --build-arg VITE_API_ENDPOINT="https://api.do.top/v1" \
    --build-arg VITE_PLAYGROUND_API_ENDPOINT="http://172.16.42.36:3001/v1" \
    "${SRC_DIR}"
echo "[${PROJECT_NAME}] 镜像编译完成"

# 推送到 Harbor
"${SCRIPT_DIR}/push_harbor.sh" "${IMAGE_NAME}" "gpu_${PROJECT_NAME}_dev" "${ACTUAL_COMMIT}"

# 重启容器
cd "${SCRIPT_DIR}" && docker compose up -d "${PROJECT_NAME}" 2>&1 | tail -3

echo "[${PROJECT_NAME}] 部署完成 (commit: ${ACTUAL_COMMIT})"
