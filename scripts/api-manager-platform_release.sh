#!/bin/bash
# api-manager-platform_release.sh — Release 构建 dev-team/api-manager-platform (Go + 前端)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="api-manager-platform"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${VERSION}"

cd "${SRC_DIR}"
echo "[api-manager-platform_release] version=${VERSION}"

# 编译前端
if [ -f "frontend/package.json" ]; then
    (cd frontend && npm ci 2>/dev/null || npm install 2>/dev/null || true)
    (cd frontend && npm run build 2>/dev/null || true)
fi

# 编译 Go 后端
CGO_ENABLED=0 go build -ldflags="-s -w" -o server ./cmd/server

# 编译 Docker 镜像（Dockerfile.local：本地先编译再打包）
docker build -t "gpu_api-manager-platform:${VERSION}" -t "gpu_api-manager-platform:latest" -f Dockerfile.local .

# 推送到 Harbor
"${SCRIPT_DIR}/push_harbor.sh" "gpu_api-manager-platform:${VERSION}" "gpu_api-manager-platform" "${VERSION}"

echo "[api-manager-platform_release] done"
