# AutoForge 구현 기획서

## Phase 0 — 기반 (1주차 상당)

### 목표
Rust 워크스페이스 빌드 가능, OmniRoute API 연동 PoC.

### 태스크

| ID | 태스크 | 완료 기준 |
|----|--------|-----------|
| P0-1 | `omniroute-client` — Chat Completions + model discovery | 모델 응답 수신 |
| P0-2 | `shared` — 도메인 타입 + 에러 | `cargo test` 통과 |
| P0-3 | `ingest` — lopdf 텍스트 추출 | 샘플 PDF → `raw_text.md` |
| P0-4 | Docker Compose (PG + Redis + MinIO) | `docker compose up` 성공 |
| P0-5 | Actix-web 서버 + PDF 업로드 UI | `cargo run` → http://localhost:8080 |

---

## Phase 1 — 단일 파이프라인 (2주차 상당)

### 목표
PDF 업로드 → 요약 → 기획까지 end-to-end (디자인/구현 제외).

### 태스크

| ID | 태스크 | 완료 기준 |
|----|--------|-----------|
| P1-1 | `orchestrator` — 상태 머신 (ingest→summarize→architect) | PostgreSQL 상태 전이 정상 |
| P1-2 | `worker` — SummarizeExecutor (Sonnet) | `summary.json` 생성 |
| P1-3 | `worker` — ArchitectExecutor (Fable, plan mode) | `architecture.md`, `spec.md` 생성 |
| P1-4 | `api` — POST `/v1/projects` (multipart PDF) | curl 업로드 → 상태 조회 |
| P1-5 | `artifacts` — S3 업로드/다운로드 | 스테이지 간 URI 전달 |

---

## Phase 2 — 디자인 + 구현 (3주차 상당)

### 목표
Stitch 디자인 + Codex 구현 + PR 자동 생성.

### 태스크

| ID | 태스크 | 완료 기준 |
|----|--------|-----------|
| P2-1 | `stitch-client` — Project 생성 + generate | 스크린 HTML/이미지 수신 |
| P2-2 | `worker` — DesignExecutor | `screens/` S3 저장 |
| P2-3 | architect ∥ design 병렬 스케줄링 | wall-clock 단축 확인 |
| P2-4 | `worker` — ImplementExecutor (Codex) | GitHub PR URL 반환 |
| P2-5 | Codex CLI에 Stitch MCP 인라인 등록 | 디자인 참조 구현 |

---

## Phase 3 — 검증 + 배포 (4주차 상당)

### 목표
프로덕션 준비: CI 검증, 재시도, 모니터링.

### 태스크

| ID | 태스크 | 완료 기준 |
|----|--------|-----------|
| P3-1 | `worker` — VerifyExecutor | CI 결과 → pass/fail |
| P3-2 | verify 실패 → Codex 재시도 루프 | 2회 재시도 후 escalate |
| P3-3 | `api` — SSE `/v1/projects/{id}/stream` | 실시간 진행률 UI |
| P3-4 | OpenTelemetry + structured logging | Datadog 대시보드 |
| P3-5 | K8s Helm chart | staging 배포 |

---

## Phase 4 — 프로덕트화

| ID | 태스크 |
|----|--------|
| P4-1 | 멀티 테넌트 (org_id, quota) |
| P4-2 | 결제 연동 (토큰 사용량 기반) |
| P4-3 | Web Dashboard (Next.js) |
| P4-4 | Slack / Discord 알림 |
| P4-5 | OmniRoute webhook v1 (run 완료 push) |

---

## 환경 변수

```bash
# 필수
OMNIROUTE_API_KEY=        # OmniRoute API key
OMNIROUTE_BASE_URL=       # Base URL reachable from the app containers
STITCH_API_KEY=           # Google Stitch
DATABASE_URL=postgres://user:pass@localhost/autoforge
REDIS_URL=redis://localhost:6379
ARTIFACTS_ENDPOINT=http://localhost:9000
ARTIFACTS_BUCKET=autoforge

# 선택
GITHUB_TOKEN=             # repo 생성/PR용
DEFAULT_REPO_TEMPLATE=    # https://github.com/org/template
OCR_ENABLED=false
OTEL_EXPORTER_OTLP_ENDPOINT=
```

---

## 모델 설정 (기본값)

```toml
# config/models.toml
[summarize]
model_id = "claude-4.6-sonnet-high-thinking"
mode = "agent"

[architect]
model_id = "claude-fable-5-thinking-high"
mode = "plan"

[implement]
model_id = "gpt-5.3-codex-high"
mode = "agent"
auto_create_pr = true

[design]
provider = "stitch"
device_type = "DESKTOP"
```

---

## 리스크 & 완화

| 리스크 | 영향 | 완화 |
|--------|------|------|
| OmniRoute API 변경 | 높음 | 호환 API 스펙 점검, adapter layer |
| Stitch 실험적 API | 중간 | HTML fallback, Figma 수동 export |
| PDF 품질 편차 | 중간 | OCR fallback, 업로드 가이드 |
| Codex 구현 품질 | 높음 | verify 루프, human review gate (optional) |
| 토큰 비용 폭주 | 높음 | 프로젝트별 spend limit, stage별 timeout |

---

## 성공 지표 (KPI)

| 지표 | 목표 |
|------|------|
| PDF → PR 완료 시간 (p50) | < 2시간 |
| 파이프라인 성공률 | > 80% |
| verify 1차 통과율 | > 60% |
| 스테이지별 평균 토큰 비용 | 프로젝트당 $15 이하 |
