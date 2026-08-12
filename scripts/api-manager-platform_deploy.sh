#!/bin/bash
# api-manager-platform_deploy.sh — 部署 dev-team/api-manager-platform (Go + 前端)
set -e

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)

echo "[api-manager-platform] commit=${COMMIT}"

# 编译前端 (如果有)
FRONTEND_DIR="$(find . -maxdepth 2 -name "package.json" -not -path "*/node_modules/*" | head -1 | xargs dirname 2>/dev/null || echo "")"
if [ -n "${FRONTEND_DIR}" ] && [ -f "${FRONTEND_DIR}/package.json" ]; then
    (cd "${FRONTEND_DIR}" && npm ci --production=false 2>/dev/null || npm install 2>/dev/null || true)
    (cd "${FRONTEND_DIR}" && npm run build 2>/dev/null || true)
fi

# 编译 Docker 镜像
docker build -t api-manager-platform:latest -f Dockerfile .

# 重启容器
docker compose up -d api-manager-platform

echo "[api-manager-platform] done"
