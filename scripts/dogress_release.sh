#!/bin/bash
# dogress_release.sh — Release 构建 dev-team/dogress (Rust Pingora LLM 代理)
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="dogress"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${VERSION}"

cd "${SRC_DIR}"
echo "[dogress_release] version=${VERSION}"

# 构建 Admin Web（admin/dist 未提交到仓库，Dockerfile COPY 需要）
if [ -f "admin/package.json" ]; then
    (cd admin && npm ci 2>/dev/null || npm install)
    (cd admin && npm run build)
fi

# 编译 Docker 镜像（bin/dogress 二进制已提交；Rust 重编译由 CI tag 阶段完成）
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' Dockerfile
docker build -t "gpu_dogress:${VERSION}" -t "gpu_dogress:latest" .

# 推送到 Harbor
"${SCRIPT_DIR}/push_harbor.sh" "gpu_dogress:${VERSION}" "gpu_dogress" "${VERSION}"

echo "[dogress_release] done"
