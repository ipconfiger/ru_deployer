#!/bin/bash
# api_deploy.sh — 部署 dev-team/api
# Rust 传入的环境变量:
#   GITLAB_PROJECT   e.g. "dev-team/api"
#   GITLAB_BRANCH    e.g. "main"
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

# 编译 Docker 镜像
IMAGE_NAME="${PROJECT_NAME}:latest"
DOCKERFILE="${SRC_DIR}/api_release/Dockerfile"
BUILD_CTX="$(dirname "${DOCKERFILE}")"

if [ ! -f "${DOCKERFILE}" ]; then
    echo "错误: 未找到 Dockerfile: ${DOCKERFILE}" >&2
    exit 1
fi

echo "[${PROJECT_NAME}] 开始编译 Docker 镜像: ${IMAGE_NAME}"
# 将 Dockerfile 中的外网 registry 地址替换为内网地址
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' "${DOCKERFILE}"
docker build -t "${IMAGE_NAME}" -f "${DOCKERFILE}" "${BUILD_CTX}"
echo "[${PROJECT_NAME}] 镜像编译完成"

# 推送到 Harbor
"${SCRIPT_DIR}/push_harbor.sh" "${IMAGE_NAME}" "gpu_${PROJECT_NAME}_dev" "${ACTUAL_COMMIT}"

# 重启容器
cd "${SCRIPT_DIR}" && docker compose up -d "${PROJECT_NAME}" 2>&1 | tail -3

echo "[${PROJECT_NAME}] 部署完成 (commit: ${ACTUAL_COMMIT})"
