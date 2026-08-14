#!/bin/bash
# dogress_deploy.sh — 部署 dev-team/dogress (Rust Pingora LLM 代理)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="dogress"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${GIT_BRANCH}"

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)
echo "[dogress] branch=${GIT_BRANCH} commit=${COMMIT}"

# 构建 Admin Web（admin/dist 未提交到仓库，Dockerfile COPY 需要）
if [ -f "admin/package.json" ]; then
    (cd admin && npm ci 2>/dev/null || npm install)
    (cd admin && npm run build)
fi

# 编译 Docker 镜像（bin/dogress 二进制已提交；镜像源 registry 替换为内网 Harbor）
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' Dockerfile
docker build -t dogress:latest .

# 重启容器
docker compose -p ru_deployer -f "${SCRIPT_DIR}/docker-compose.yml" up -d dogress

echo "[dogress] done"
