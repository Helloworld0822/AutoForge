# Podman / Docker Compose 가이드

AutoForge는 **`compose.yml` 하나**로 Docker Compose와 Podman Compose를 모두 지원합니다.  
컨테이너 빌드는 **`Containerfile`** (Dockerfile 호환)을 사용합니다.

## 아키텍처

```
  브라우저 ──► nginx:80 ──┬── / (React SPA)
                          ├── /v1/* ──► api:8080
                          └── /health ──► api:8080

  api / orchestrator / worker ──► Redis Streams MQ
              │
      공유 볼륨 artifacts-data (파이프라인 산출물 + 이미지 호스팅)
```

| 서비스 | 역할 |
|--------|------|
| **nginx** | 프론트엔드 정적 파일 + API 리버스 프록시 |
| **api** | Rust REST API (`backend/`) |
| **orchestrator** | 이벤트 소비 → 다음 스테이지 enqueue |
| **worker** | 커맨드 소비 → 스테이지 실행 (수평 확장) |
| **redis** | MQ + 프로젝트 상태 |

아티팩트 저장소는 별도 서비스 없이 `artifacts-data` 공유 볼륨(로컬 디스크)을 사용합니다.
api/orchestrator/worker 컨테이너가 모두 같은 볼륨을 `/data/artifacts`에 마운트하므로
스테이지 산출물과 업로드 이미지가 프로세스 간에 공유됩니다.

## 메시지 큐 최적화

| 스트림 | 용도 | Consumer |
|--------|------|----------|
| `autoforge:commands` | 스테이지 실행 커맨드 | Worker (수평 확장) |
| `autoforge:events` | 완료/실패 이벤트 | Orchestrator |

**최적화 포인트:**
- Worker 3+ replica로 병렬 스테이지 처리 (`architect` ∥ `design`)
- `WORKER_CONCURRENCY`로 컨테이너당 동시 스테이지 실행 수 제어
- Redis Consumer Group으로 at-least-once + 자동 재분배
- API는 커맨드 enqueue만 — 블로킹 없음

## 빠른 시작

```bash
cp .env.example .env
# .env 편집

chmod +x scripts/compose-up.sh scripts/podman-up.sh

# Docker
./scripts/compose-up.sh

# Podman (rootless — localhost/ 이미지 태그 자동 적용)
./scripts/podman-up.sh
```

수동 실행:

```bash
# Docker
docker compose up -d --build --scale worker=3

# Podman (Compose v2)
IMAGE_PREFIX=localhost/ podman compose up -d --build --scale worker=3

# Podman (legacy podman-compose)
IMAGE_PREFIX=localhost/ podman-compose -f compose.yml up -d --build --scale worker=3
```

접속: **http://localhost:8080** (Podman rootless 기본)

### rootless에서 80번 포트가 안 되는 이유

Linux rootless 컨테이너는 **1024 미만 포트(80, 443 등)에 바인딩할 수 없습니다.**  
`compose-up.sh` / `podman-up.sh`는 자동으로 `HOST_HTTP_PORT=8080`을 사용합니다.

**80번을 꼭 쓰려면** (시스템 관리자 권한 필요):

```bash
# 현재 세션만
sudo sysctl -w net.ipv4.ip_unprivileged_port_start=80

# 영구 적용 (/etc/sysctl.d/99-unprivileged-ports.conf)
echo 'net.ipv4.ip_unprivileged_port_start=80' | sudo tee /etc/sysctl.d/99-unprivileged-ports.conf
sudo sysctl --system

# 이후 80 사용 가능
HOST_HTTP_PORT=80 ./scripts/podman-up.sh
```

Docker 데몬(rootful) 사용 시: **http://localhost** (`HOST_HTTP_PORT=80` 기본)

## Containerfile

| 경로 | 이미지 |
|------|--------|
| `backend/Containerfile` | `autoforge-api` (Rust 멀티 스테이지) |
| `nginx/Containerfile` | `autoforge-nginx` (React 빌드 + nginx) |

Podman과 Docker 모두 `dockerfile: Containerfile` 필드로 빌드합니다 (Compose 표준 키 이름).

## 이미지 태그 (`IMAGE_PREFIX`)

| 런타임 | `IMAGE_PREFIX` | 예시 태그 |
|--------|----------------|-----------|
| Docker | (비움) | `autoforge-api:latest` |
| Podman rootless | `localhost/` | `localhost/autoforge-api:latest` |

`./scripts/podman-up.sh`는 `IMAGE_PREFIX=localhost/`를 자동 설정합니다.

## Worker 스케일링

```bash
WORKER_SCALE=5 ./scripts/compose-up.sh

# Podman
WORKER_SCALE=5 ./scripts/podman-up.sh
```

특정 스테이지만 처리:

```bash
podman run --rm -e STAGE_FILTER=implement localhost/autoforge-api:latest worker
```

## Compose 엔진 자동 감지

`scripts/compose-up.sh`는 다음 순서로 엔진을 선택합니다:

1. `docker compose`
2. `podman compose`
3. `podman-compose` (legacy)

## 로컬 개발 (MQ 없이)

```bash
MESSAGE_QUEUE_ENABLED=false cargo run
```

단일 프로세스 인라인 모드로 실행됩니다.

## 환경 변수

| 변수 | 기본값 | 설명 |
|------|--------|------|
| `MESSAGE_QUEUE_ENABLED` | `true` (Compose) | MQ 모드 활성화 |
| `REDIS_URL` | `redis://redis:6379` | Redis 연결 |
| `IMAGE_PREFIX` | (비움) | Podman rootless 시 `localhost/` |
| `WORKER_SCALE` | `3` | worker replica 수 (`compose-up.sh`) |

자세한 변수는 [.env.example](../.env.example) 참고.
