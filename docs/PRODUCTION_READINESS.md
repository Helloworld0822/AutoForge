# 실사용 전환 작업 기록

이 문서는 AutoForge를 데모/PoC 수준에서 실사용 가능한 수준으로 끌어올리기 위해
**데이터 영속성 → 분산 처리 정확성 → 보안 → 운영 도구** 순서로 진행한 개선 사항과,
아직 남아있는 과제를 정리합니다.

> **참고 (후속 변경)**: 아래 1절에서는 최초에 MinIO/S3(`rust-s3`)로 아티팩트 스토어를
> 구현했다고 기록되어 있으나, 이후 요청에 따라 **외부 오브젝트 스토리지 의존성을 제거**하고
> 로컬 디스크 기반 `LocalArtifactStore`로 교체했습니다 (`ARTIFACTS_DIR`). 동시에 별도
> 인프라 없이 사용 가능한 **이미지 호스팅 기능**(`/v1/images`, `/media/{filename}`)을
> 추가했습니다. Compose 환경에서는 `artifacts-data` 공유 볼륨으로 api/worker/orchestrator
> 간 파일을 공유합니다. 현재 코드 기준으로는 MinIO/S3 관련 서술을 무시하고 이 안내를
> 따르세요.

## 1. 데이터 영속성

| 변경 사항 | 파일 |
|-----------|------|
| MinIO/S3 아티팩트 스토어를 실제 연동 (`rust-s3` 클라이언트, path-style, 버킷 자동 생성) | `backend/src/services/artifacts.rs` |
| 연결 실패/타임아웃(8초) 시 인메모리로 자동 폴백 + 경고 로그 (`is_durable()`로 상태 노출) | 〃 |
| Redis 프로젝트 목록 조회를 `KEYS`(블로킹) → `SCAN` 커서 방식으로 변경 | `backend/src/services/store/redis_store.rs` |
| 손상된 레코드는 건너뛰고 경고 로그 (이전에는 조용히 무시) | 〃 |
| PDF/DevOps 계획서 원본 바이트를 아티팩트 업로드 후 `Project`에서 제거 → Redis에 대용량 바이너리 중복 저장 방지 | `backend/src/services/pipeline/engine.rs` |
| Redis/MQ 연결에 10초 타임아웃 적용 — 연결 불가 시 기동이 무한 대기하지 않고 즉시 에러 반환 | `backend/src/services/store/redis_store.rs`, `backend/src/services/queue/mod.rs` |
| **(버그 수정)** `ArtifactRef`에 `key` 필드 분리 추가 — 기존에는 표시용 이름(`plan.pdf`)과 실제 저장 키(`projects/{id}/plan.pdf`)가 같은 필드에 섞여 있어 `IngestExecutor`가 잘못된 키로 조회를 시도하는 문제가 있었음 | `backend/src/domain/types.rs` 외 |
| **(버그 수정)** `MESSAGE_QUEUE_ENABLED` 미설정 시 `REDIS_URL` 기본값이 항상 채워져 있어 로컬 단일 프로세스 모드도 Redis 연결을 시도해 기동이 무한 대기하던 문제 — 이제 명시적 플래그로만 판단 | `backend/src/config.rs` |

## 2. 분산 처리 정확성

| 변경 사항 | 파일 |
|-----------|------|
| MQ 모드에서 `StageCompleted` 이벤트에 실제 아티팩트 목록을 포함 (기존에는 빈 배열로 재구성되어 하위 스테이지가 입력을 받지 못함) | `backend/src/services/queue/messages.rs`, `backend/src/services/pipeline/mq.rs` |
| 오케스트레이터가 이벤트 처리에 **성공한 경우에만 ACK** — 실패 시 pending 상태로 남겨 재시도 가능 | `backend/src/services/pipeline/mq.rs` |
| `XAUTOCLAIM` 기반 stale 메시지 재소유(reclaim) — 컨슈머 크래시 및 반복 실패 복구 | `backend/src/services/queue/mod.rs` |
| 최대 재시도(5회) 초과 시 데드레터 스트림(`{stream}:dlq`)으로 이동 후 ACK — 무한 재처리 방지 | 〃 |
| 워커 단에서 멱등성 처리: 이미 `Completed` 상태인 스테이지 커맨드/이벤트는 중복 실행하지 않고 스킵 | `backend/src/services/pipeline/mq.rs` |
| `Cancelled` 상태 확인 후 워커/오케스트레이터/인라인 실행 루프가 실제로 작업을 중단 (이전에는 API가 상태만 바꾸고 실행 중인 파이프라인은 계속 진행됨) | `backend/src/services/pipeline/mq.rs`, `backend/src/services/pipeline/engine.rs` |

## 3. 보안

| 변경 사항 | 파일 |
|-----------|------|
| `API_KEY` 설정 시 `/v1/*`에 `Authorization: Bearer` 인증 미들웨어 적용 | `backend/src/web/auth.rs`, `backend/src/web/routes.rs` |
| `CORS_ALLOWED_ORIGINS` 설정 시 오리진 화이트리스트 적용 (미설정 시 경고 로그와 함께 모든 오리진 허용) | `backend/src/web/mod.rs` |
| 업로드 크기 제한(`MAX_UPLOAD_BYTES`, 기본 50MB)을 애플리케이션 레벨에서 강제 (기존엔 nginx 레벨만 존재) | `backend/src/web/handlers.rs` |
| `name`/`repo_url` 등 입력 필드 검증 (길이 제한, GitHub URL 형식 검사, 빈 PDF 거부) | 〃 |
| 기동 시 설정 검증(`validate_and_warn`) — 필수 API 키 누락, 인증 비활성화, GitHub 토큰 형식, PUBLIC_URL 등을 경고 | `backend/src/config.rs` |
| nginx에 기본 rate limit (`limit_req_zone`, IP당 5 req/s) 및 HTTPS 서버 블록 템플릿(주석) 추가 | `nginx/nginx.conf` |
| 백엔드 Dockerfile: non-root 사용자로 실행 | `backend/Dockerfile` |
| MinIO 콘솔/기본 자격증명 관련 운영 체크리스트 문서화 | `README.md` |

## 4. 운영 도구

| 변경 사항 | 파일 |
|-----------|------|
| `/ready` readiness probe 추가 — store/artifacts/queue 연결 상태를 점검하고 비정상 시 503 반환 | `backend/src/web/handlers.rs` |
| `/health`에 `auth_enabled` 등 상태 필드 추가 | 〃 |
| GitHub Actions CI 파이프라인: 백엔드(`fmt`, `clippy -D warnings`, `build`, `test`), 프론트엔드(`lint`, `build`), Docker Compose 스모크 테스트 | `.github/workflows/ci.yml` |
| `docker-compose.yml`/`podman-compose.yml`에 전 서비스 헬스체크 추가 (api/nginx/redis/minio), `depends_on: condition: service_healthy`로 기동 순서 보장 | `docker-compose.yml`, `podman-compose.yml` |
| 서비스별 CPU/메모리 리소스 제한(`deploy.resources.limits`) 추가 | 〃 |
| 코드 전체 `cargo fmt` 정리 + `cargo clippy -D warnings` 통과 (기존 미준수 항목 다수 수정) | 전체 |

## 5. 아직 남은 과제 (우선순위순)

이번 작업에서 다루지 않은 항목들입니다. 필요 시 후속 작업으로 진행하세요.

1. **PostgreSQL 마이그레이션 정리** — `migrations/001_init.sql`이 존재하지만 실제 코드에서 사용되지 않음. Redis만으로 충분한지, 관계형 저장소가 필요한지 결정 후 정리.
2. **실시간 SSE 스트리밍** — 현재 `/v1/projects/{id}/stream`은 스냅샷 1건만 보내고 종료됨. 프론트엔드는 폴링으로 대체 중. 실제 서버-푸시가 필요하면 pub/sub(Redis) 기반으로 구현 필요.
3. **오케스트레이터 리더 선출** — 오케스트레이터를 복수 실행하면 중복 스케줄링 위험. 단일 인스턴스로 운영하거나 분산 락(Redis `SET NX` 등) 도입 필요.
4. **TLS 인증서 자동화** — nginx에 HTTPS 템플릿은 추가했으나 Let's Encrypt/Certbot 자동 갱신은 미포함.
5. **GitHub Enterprise / 다중 PR 지원** — 현재 `github.com` 호스트만 파싱, 열린 PR이 여러 개면 최신 것만 병합.
6. **Slack 스레드 업데이트** — Webhook 모드에서는 스레드 갱신 불가 (Bot Token 필요), 일별 다이제스트가 이벤트마다 재전송되어 스팸 위험.
7. **백업/복구 절차** — Redis/MinIO 볼륨에 대한 백업 스크립트나 문서 없음.
8. **통합/E2E 테스트** — 현재는 단위 테스트(15개) + CI의 compose 스모크 테스트만 존재. 실제 파이프라인 흐름(ingest→summarize→...)에 대한 통합 테스트는 없음 (외부 AI API 의존성 때문에 모킹 필요).
