#!/bin/bash
# flint_release.sh — Release 构建 dev-team/flint
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="flint"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${VERSION}"

cd "${SRC_DIR}"
echo "[flint_release] version=${VERSION}"

# 编译 Docker 镜像
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' flint/Dockerfile
sed -i 's|FROM debian:bookworm-slim|FROM ubuntu:24.04|g' flint/Dockerfile
docker build -t "gpu_flint:${VERSION}" -t "gpu_flint:latest" -f flint/Dockerfile flint/

# 推送到 Harbor
"${SCRIPT_DIR}/push_harbor.sh" "gpu_flint:${VERSION}" "gpu_flint" "${VERSION}"

echo "[flint_release] done"
