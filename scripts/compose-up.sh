#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

COMPOSE_FILE="${COMPOSE_FILE:-compose.yml}"
WORKER_SCALE="${WORKER_SCALE:-3}"

podman_socket() {
  echo "unix:///run/user/$(id -u)/podman/podman.sock"
}

docker_ready() {
  command -v docker &>/dev/null \
    && docker compose version &>/dev/null 2>&1 \
    && docker info &>/dev/null 2>&1
}

podman_ready() {
  command -v podman &>/dev/null && podman info &>/dev/null 2>&1
}

podman_rootless() {
  podman_ready && podman info --format '{{.Host.Security.Rootless}}' 2>/dev/null | grep -q true
}

# rootless Podman은 1024 미만 포트(80) 바인딩 불가 → 8080으로 자동 전환
apply_podman_port_defaults() {
  if ! podman_rootless; then
    return
  fi

  if [[ -z "${HOST_HTTP_PORT:-}" || "${HOST_HTTP_PORT}" == "80" ]]; then
    export HOST_HTTP_PORT=8080
    echo "NOTE: rootless Podman cannot bind port 80 — using HOST_HTTP_PORT=8080"
    echo "      For port 80: sudo sysctl -w net.ipv4.ip_unprivileged_port_start=80"
    echo "      Or always use: HOST_HTTP_PORT=8080 ./scripts/podman-up.sh"
    echo ""
  fi

  if [[ "${PUBLIC_URL:-http://localhost}" == "http://localhost" ]]; then
    export PUBLIC_URL="http://localhost:${HOST_HTTP_PORT}"
  fi
}

compose_engine() {
  if docker_ready; then
    echo "docker"
  elif podman_ready; then
    echo "podman"
  elif command -v podman-compose &>/dev/null; then
    echo "podman-compose"
  else
    return 1
  fi
}

run_compose() {
  local engine
  engine="$(compose_engine)" || {
    echo "docker daemon, podman, or podman-compose required"
    exit 1
  }

  case "$engine" in
    docker)
      docker compose -f "$COMPOSE_FILE" "$@"
      ;;
    podman)
      DOCKER_HOST="$(podman_socket)" docker compose -f "$COMPOSE_FILE" "$@"
      ;;
    podman-compose)
      podman-compose -f "$COMPOSE_FILE" "$@"
      ;;
  esac
}

engine="$(compose_engine)" || {
  echo "docker daemon, podman, or podman-compose required"
  exit 1
}

if [[ "$engine" == "podman" || "$engine" == "podman-compose" ]]; then
  apply_podman_port_defaults
fi

echo "==> Building and starting AutoForge stack (${engine}, ${COMPOSE_FILE})"
run_compose up -d --build --scale "worker=${WORKER_SCALE}"

echo ""
echo "AutoForge is running:"
echo "  Web (nginx):  http://localhost:${HOST_HTTP_PORT:-8080}"
echo "  API (proxy):  http://localhost:${HOST_HTTP_PORT:-8080}/v1"
echo "  Images:       http://localhost:${HOST_HTTP_PORT:-8080}/media/{filename}"
echo ""
echo "Scale workers: WORKER_SCALE=5 ./scripts/compose-up.sh"
echo "Podman (rootless): ./scripts/podman-up.sh"
