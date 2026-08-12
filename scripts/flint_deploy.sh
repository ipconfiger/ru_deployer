#!/bin/bash
# flint_deploy.sh — 部署 dev-team/flint
set -e

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)

echo "[flint] commit=${COMMIT}"

# 编译 Docker 镜像
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' flint/Dockerfile
sed -i 's|FROM debian:bookworm-slim|FROM ubuntu:24.04|g' flint/Dockerfile
docker build -t flint:latest -f flint/Dockerfile flint/

# 重启容器
docker compose up -d flint

echo "[flint] done"
