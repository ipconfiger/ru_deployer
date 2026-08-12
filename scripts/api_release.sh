#!/bin/bash
# api_release.sh — Release 构建 dev-team/api
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="api"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${VERSION}"

cd "${SRC_DIR}"
echo "[api_release] version=${VERSION}"

# 编译镜像
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' api_release/Dockerfile
docker build -t "gpu_api:${VERSION}" -f api_release/Dockerfile api_release/

# 推送到 Harbor
"${SCRIPT_DIR}/push_harbor.sh" "gpu_api:${VERSION}" "gpu_api" "${VERSION}"

echo "[api_release] done"
