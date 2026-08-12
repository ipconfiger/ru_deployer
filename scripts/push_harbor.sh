#!/bin/bash
# push_harbor.sh — 推送镜像到 Harbor（best-effort，失败不阻塞调用方）
#
# 用法: push_harbor.sh <source_image> <harbor_image_name> <version>
# 示例: push_harbor.sh "api:latest" "gpu_api_dev" "abc1234"
#
# 环境变量:
#   HARBOR_REGISTRY   Harbor 地址        (默认 172.16.29.88:30800)
#   HARBOR_PROJECT    Harbor 项目名      (默认 cloud-platform)
#   HARBOR_USER       Harbor 用户名      (默认 admin)
#   HARBOR_PASSWORD   Harbor 密码        (必须; 为空则跳过)
#
# 退出码: 0 = 成功/跳过, 非 0 = 参数错误

set -euo pipefail

# === 配置（可用环境变量覆盖）===
HARBOR_REGISTRY="${HARBOR_REGISTRY:-172.16.29.88:30800}"
HARBOR_PROJECT="${HARBOR_PROJECT:-gpu}"
HARBOR_USER="${HARBOR_USER:-robot\$gpu+gpubot}"

SOURCE_IMAGE="$1"
HARBOR_IMAGE_NAME="$2"
VERSION="$3"

if [ -z "${SOURCE_IMAGE}" ] || [ -z "${HARBOR_IMAGE_NAME}" ] || [ -z "${VERSION}" ]; then
    echo "用法: $0 <source_image> <harbor_image_name> <version>" >&2
    echo "示例: $0 api:latest gpu_api_dev abc1234" >&2
    exit 1
fi

# 无密码则静默跳过
if [ -z "${HARBOR_PASSWORD:-}" ]; then
    echo "[push_harbor] ⚠ 未设置 HARBOR_PASSWORD，跳过" >&2
    exit 0
fi

HARBOR_IMAGE="${HARBOR_REGISTRY}/${HARBOR_PROJECT}/${HARBOR_IMAGE_NAME}"

echo "[push_harbor] 登录 ${HARBOR_REGISTRY}..."
if ! echo "${HARBOR_PASSWORD}" | docker login "${HARBOR_REGISTRY}" -u "${HARBOR_USER}" --password-stdin > /dev/null 2>&1; then
    echo "[push_harbor] ⚠ 登录失败，跳过推送" >&2
    exit 0
fi

echo "[push_harbor] 推送: ${SOURCE_IMAGE} -> ${HARBOR_IMAGE}:${VERSION}, ${HARBOR_IMAGE}:latest"

docker tag "${SOURCE_IMAGE}" "${HARBOR_IMAGE}:${VERSION}"
docker tag "${SOURCE_IMAGE}" "${HARBOR_IMAGE}:latest"

if docker push "${HARBOR_IMAGE}:${VERSION}" && docker push "${HARBOR_IMAGE}:latest"; then
    echo "[push_harbor] 推送完成: ${HARBOR_IMAGE}"
else
    echo "[push_harbor] ⚠ 推送失败" >&2
fi

exit 0
