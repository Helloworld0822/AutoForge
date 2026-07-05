#!/usr/bin/env bash
# Podman 래퍼 — rootless 이미지 태그 + 비특권 포트 적용
set -euo pipefail
export IMAGE_PREFIX="${IMAGE_PREFIX:-localhost/}"
export HOST_HTTP_PORT="${HOST_HTTP_PORT:-8080}"
export COMPOSE_FILE="${COMPOSE_FILE:-compose.yml}"
exec "$(dirname "$0")/compose-up.sh" "$@"
