#!/usr/bin/env bash
# AutoForge — VPS 원격 배포 스크립트 (CD 파이프라인에서 SSH로 호출)
#
# VPS(Docker Compose)에서 실행되어 ghcr.io에서 새 이미지를 pull하고 스택을 재시작한다.
# DB/세션/아티팩트 상태는 파괴하지 않는다(재생성 없음).
#
# 사전 요구사항 (VPS):
#   * Docker Engine + docker compose plugin 설치
#   * 배포 디렉터리에 compose.yml 파일 존재
#   * ./compose.prod.yml  (생략 시 pull은 암시되나 안전상 권장)
#   * ./.env  (민감값 — 로그인/API 키 등. 서버에 고정 보관)
#   * production compose overlay는 이미지에 pull_policy: always 를 지정하므로
#     별도 `docker image pull` 없이도 `up -d`가 항상 최신 이미지를 받아온다.
#
# 사용법:
#   DEPLOY_DIR=/opt/autoforge IMAGE_PREFIX=ghcr.io/<owner>/ ./scripts/remote-deploy.sh
#
# 환경 변수:
#   DEPLOY_DIR     compose 파일이 있는 디렉터리 (기본: /opt/autoforge)
#   IMAGE_PREFIX   ghcr.io 앞 prefix (예: ghcr.io/your-org/) — compose.prod.yml 과 함께 사용
#   COMPOSE_PROD   production overlay 파일명 (기본: compose.prod.yml)
#   PRUNE_IMAGES   "1"이면 24h 경과 이미지 자동 정리 (기본: 정리 안 함 — 롤백용 이전 태그 보존)
set -euo pipefail

DEPLOY_DIR="${DEPLOY_DIR:-/opt/autoforge}"
COMPOSE_PROD="${COMPOSE_PROD:-compose.prod.yml}"
IMAGE_PREFIX="${IMAGE_PREFIX:-ghcr.io/}"
PRUNE_IMAGES="${PRUNE_IMAGES:-0}"

cd "${DEPLOY_DIR}"

if [[ ! -f compose.yml ]]; then
  echo "ERROR: compose.yml not found in ${DEPLOY_DIR}" >&2
  exit 1
fi

if [[ ! -f "${COMPOSE_PROD}" ]]; then
  echo "WARN: ${COMPOSE_PROD} not found — deploying without pull_policy=always; existing images will be reused." >&2
fi

COMPOSE_ARGS=(docker compose)
COMPOSE_ARGS+=(-f compose.yml)
if [[ -f "${COMPOSE_PROD}" ]]; then
  COMPOSE_ARGS+=(-f "${COMPOSE_PROD}")
fi

echo "==> [$(date -u +%Y-%m-%dT%H:%M:%SZ)] Deploying AutoForge (${DEPLOY_DIR})"
echo "    image prefix: ${IMAGE_PREFIX}"

export IMAGE_PREFIX

# 1. 최신 이미지 명시적으로 pull (권장 — overlay가 항상 최신을 받지만 이중 보호)
"${COMPOSE_ARGS[@]}" pull --quiet

# 2. 변경된 서비스 재생성 (volumes/DB 유지)
"${COMPOSE_ARGS[@]}" up -d --remove-orphans

# 3. 이전 컨테이너 이미지 정리 (선택) — 롤백하려면 PRUNE_IMAGES=0 유지
if [[ "${PRUNE_IMAGES}" == "1" ]]; then
  docker image prune -f --filter "until=24h" || true
fi

echo "==> Deploy complete."
echo ""
"${COMPOSE_ARGS[@]}" ps
