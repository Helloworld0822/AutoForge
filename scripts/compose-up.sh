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

# podman-compose가 만든 네트워크는 com.docker.compose.network 라벨이 없어
# docker compose (DOCKER_HOST=podman.sock) 실행 시 실패한다.
repair_mislabeled_compose_network() {
  local engine="$1"
  if [[ "$engine" != "podman" ]]; then
    return 0
  fi

  local network_name project_name label
  network_name="$(
    DOCKER_HOST="$(podman_socket)" docker compose -f "$COMPOSE_FILE" config --format json 2>/dev/null \
      | python3 -c "import json,sys; c=json.load(sys.stdin); print(c.get('networks',{}).get('default',{}).get('name', c.get('name','') + '_default'))"
  )" || return 0
  [[ -n "$network_name" ]] || return 0

  if ! podman network exists "$network_name" &>/dev/null; then
    return 0
  fi

  label="$(podman network inspect "$network_name" --format '{{index .Labels "com.docker.compose.network"}}' 2>/dev/null || true)"
  if [[ "$label" == "default" ]]; then
    return 0
  fi

  project_name="${network_name%_default}"
  echo "NOTE: network ${network_name} has stale compose labels (com.docker.compose.network=\"${label}\", expected \"default\")"
  echo "==> Removing stale Podman Compose resources for project ${project_name}..."

  podman ps -aq --filter "network=${network_name}" 2>/dev/null | xargs -r podman rm -f
  podman pod rm -f "pod_${project_name}" 2>/dev/null || true
  podman network rm -f "$network_name"
}

run_compose() {
  local engine
  engine="$(compose_engine)" || {
    echo "docker daemon, podman, or podman-compose required"
    exit 1
  }

  local compose_args=(-f "$COMPOSE_FILE")
  local adc_path="${HOST_GCLOUD_ADC:-${HOME}/.config/gcloud/application_default_credentials.json}"
  if [[ -f "$adc_path" ]]; then
    export HOST_GCLOUD_ADC="$adc_path"
    compose_args+=(-f compose.gcloud.yml)
    echo "==> Mounting gcloud ADC for Stitch Bearer auto-refresh"
  else
    echo "NOTE: No gcloud ADC at ${adc_path} — set a fresh STITCH_ACCESS_TOKEN or run: gcloud auth application-default login"
  fi

  case "$engine" in
    docker)
      docker compose "${compose_args[@]}" "$@"
      ;;
    podman)
      DOCKER_HOST="$(podman_socket)" docker compose "${compose_args[@]}" "$@"
      ;;
    podman-compose)
      podman-compose "${compose_args[@]}" "$@"
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

repair_mislabeled_compose_network "$engine"

bootstrap_rabbitmq_data() {
  if [[ "$engine" != "podman" && "$engine" != "podman-compose" ]]; then
    return 0
  fi

  local data_dir="${RABBITMQ_DATA_DIR:-data/rabbitmq}"
  mkdir -p "$data_dir"

  # Fresh bind mount: rootless Podman copy-up can leave .erlang.cookie owned by the host user.
  if [[ -d "$data_dir/mnesia" ]]; then
    return 0
  fi

  echo "==> Bootstrapping RabbitMQ data in ${data_dir} (rootless Podman)..."
  podman run --rm --user 0:0 \
    -v "$(pwd)/${data_dir}:/var/lib/rabbitmq" \
    docker.io/library/rabbitmq:3-management-alpine \
    sh -c 'rabbitmq-server -detached; for i in $(seq 1 30); do rabbitmq-diagnostics -q ping && exit 0; sleep 1; done; exit 1'
}

bootstrap_rabbitmq_data

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
