#!/bin/bash
# flint_deploy.sh — 部署 dev-team/flint
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="flint"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${GIT_BRANCH}"

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)
echo "[flint] branch=${GIT_BRANCH} commit=${COMMIT}"

# 编译 Docker 镜像
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' flint/Dockerfile
sed -i 's|FROM debian:bookworm-slim|FROM ubuntu:24.04|g' flint/Dockerfile
docker build -t flint:latest -f flint/Dockerfile flint/

# 重启容器 (部署到测试机时取消注释)
# docker compose up -d flint

echo "[flint] done"
