# AutoForge — AI 외주 자동화 프로그램

PDF 계획서를 로컬 Markdown으로 변환하고, **OmniRoute로 요구사항 추출·계획·상위 디버그 진단**을 실행합니다. 구현·검증·수정·보안 패치는 현재 **Cursor Cloud Agent**가 담당하는 하이브리드 구조입니다. RabbitMQ worker와 기존 GitHub delivery 흐름을 유지합니다.

> 전체 V2 마이그레이션이 완료된 것은 아닙니다. 로컬 patch executor, task별 context 연결, 강제 비용 예산은 아직 미완성입니다. [구조 평가와 후속 계획](docs/STRUCTURE_REVIEW.md)을 확인하세요.

## OmniRoute 연결과 토큰 절감

- `OMNIROUTE_BASE_URL` 기본값: `http://127.0.0.1:20128/v1`. `/chat/completions`, `/models`를 이 주소에 붙입니다.
- `OMNIROUTE_API_KEY`와 역할별 `OMNIROUTE_MODEL_*`를 설정하세요. 실제 인스턴스의 `/v1/models`에서 확인한 ID만 사용합니다. 추측한 모델 기본값은 없습니다.
- 이전 `OPENROUTER_*`/`OMNIROUTER_*` 변수는 더 이상 읽지 않습니다. 운영 설정을 `OMNIROUTE_*`로 옮겨야 합니다.
- `pdf-inspector` compact Markdown, 페이지 참조, native `pdftotext` fallback을 사용합니다. 스캔/혼합 PDF는 OCR이 구성되지 않았다는 오류로 중단하며 부분 추출을 성공 처리하지 않습니다.
- 요구사항 JSON은 축약 요약이 아닌 구조화 추출입니다. 스키마 오류만 최대 2회 호출하고, 같은 프로젝트에서 문서·모델·출력 설정이 같으면 저장된 추출 결과를 재사용합니다.
- 계획 입력은 compact JSON, 디자인 입력은 UI 요구사항만, 구현 입력은 허용된 산출물 URI만 전송합니다. 디버그에는 실제 검증 로그의 중복 제거·크기 제한 결과를 넣습니다.
- `AI_MAX_INPUT_TOKENS`는 토크나이저 독립 추정치의 상한입니다. 초과 시 원문을 잘라 보내지 않고 실패합니다. 출력 상한은 추출/계획 16K, 질문 2K, 진단 4K이며 설정 가능합니다.

절감률은 문서·언어·모델에 따라 다릅니다. 추정 토큰과 실제 provider usage를 혼동하지 않으며, 실과금 절감률은 아직 측정하지 않았습니다.

## 프로젝트 구조

```
backend/          # Rust API (Actix-web)
frontend/         # React + Vite + TypeScript UI
nginx/            # Nginx 리버스 프록시 설정
compose.yml       # Docker / Podman Compose (통합)
backend/Containerfile
nginx/Containerfile
```

## 아키텍처 (Compose)

```
                    ┌─────────────┐
  브라우저 ────────►│   nginx:80  │
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
         / (SPA)      /v1/*       /health
              │            │            │
              ▼            ▼            ▼
        [정적 파일]    api:8080    api:8080
                           │
                    orchestrator + worker×N
                           │
                    redis (store) + rabbitmq (MQ) + artifacts-data(volume)
```

- **nginx**: 프론트엔드 정적 파일 서빙 + API 프록시
- **api**: Rust REST API (프론트엔드 정적 파일 미포함)
- **orchestrator / worker**: RabbitMQ 기반 분산 파이프라인

## 빠른 시작 (Docker / Podman Compose)

```bash
cp .env.example .env
# .env 편집 (OMNIROUTE_API_KEY 및 OmniRoute gateway, GITHUB_TOKEN 등)
# OMNIROUTE_MODEL_EXTRACT / PLAN 등을 실제 모델 목록에서 선택
# 호스트에서 OmniRoute를 실행하고 Compose를 사용할 경우:
# OMNIROUTE_BASE_URL=http://host.containers.internal:20128/v1

# Docker
./scripts/compose-up.sh
# 또는: docker compose up -d --build --scale worker=3

# Podman (rootless) — IMAGE_PREFIX는 podman-up.sh가 자동 설정
./scripts/podman-up.sh
# 또는: cp .env.example .env 후 podman compose up -d --build --scale worker=3

open http://localhost:8080
```

| URL | 설명 |
|-----|------|
| http://localhost:8080 | React 대시보드 (nginx, 기본 포트) |
| http://localhost:8080/v1/projects | API |
| http://localhost:8080/media/{filename} | 업로드된 이미지 (이미지 호스팅) |

## 로컬 개발 (분리 실행)

```bash
# 터미널 1 — 백엔드 API
# Rust >= 1.89, pdftotext fallback에는 poppler-utils 필요
cd backend && cargo run

# 터미널 2 — 프론트엔드 (API :8080 프록시)
cd frontend && npm install && npm run dev
# http://localhost:5173
```

## GitHub 자동화

`GITHUB_TOKEN` 설정 시 프라이빗 레포 자동 생성 → 기존 Cursor workspace executor의 PR 생성 → SecurityPatch 통과 후 자동 merge. AI 추출·계획·진단은 OmniRoute로 라우팅됩니다.

```bash
export GITHUB_TOKEN=ghp_xxxx
export GITHUB_ORG=my-org        # 선택
export GITHUB_AUTO_MERGE=true   # 기본값
```

## API 엔드포인트

| Method | Path | 설명 | 인증 |
|--------|------|------|------|
| GET | `/health` | Liveness (프로세스 생존) | ❌ |
| GET | `/ready` | Readiness (스토어/Redis/Cursor/Stitch 프로브) | ❌ |
| POST | `/v1/projects` | PDF 업로드 + DevOps 계획서(선택) + 파이프라인 시작 | ✅ |
| GET | `/v1/projects` | 프로젝트 목록 | ✅ |
| GET | `/v1/projects/{id}` | 프로젝트 상세 | ✅ |
| GET | `/v1/projects/{id}/stream` | SSE 진행률 | ✅ |
| POST | `/v1/projects/{id}/cancel` | 파이프라인 취소 | ✅ |
| GET | `/v1/projects/{id}/daily-logs` | 일별 경과 목록 | ✅ |
| GET | `/v1/projects/{id}/daily-logs/{date}` | 특정 날짜 MD 로그 | ✅ |
| POST | `/v1/images` | 이미지 업로드 (필드명: `image`) → 호스팅 URL 반환 | ✅ |
| GET | `/v1/images` | 업로드된 이미지 목록 (최신순) | ✅ |
| GET | `/media/{filename}` | 업로드된 이미지 직접 서빙 (공유/임베드용) | ❌ |

Compose 환경에서는 nginx가 `/v1`, `/health`, `/ready`, `/media`를 `api:8080`으로 프록시합니다.

`API_KEY` 환경변수를 설정하면 `/v1/*`는 `Authorization: Bearer <API_KEY>` 헤더가 필요합니다.
미설정 시 인증 없이 열려있으므로(로컬 개발 편의) **운영 환경에서는 반드시 설정하세요.**
`/media/{filename}`은 외부 공유·임베드를 위해 인증 없이 접근 가능합니다.

### 이미지 호스팅

PDF/DevOps 계획서와 별개로, 코드 생성과 무관하게 이미지를 업로드해 바로 접근 가능한
URL을 받을 수 있는 간단한 이미지 호스팅 기능을 제공합니다. 별도 오브젝트 스토리지 없이
로컬 디스크(`ARTIFACTS_DIR`)에 저장되며, PNG/JPG/GIF/WEBP/BMP/SVG를 지원합니다.

```bash
curl -X POST http://localhost/v1/images \
  -H "Authorization: Bearer $API_KEY" \
  -F "image=@screenshot.png"
# => {"filename":"<uuid>.png","url":"http://localhost/media/<uuid>.png","content_type":"image/png"}
```

프론트엔드 대시보드의 **이미지 호스팅** 탭에서도 업로드/링크 복사가 가능합니다.

`WORKER_CONCURRENCY`로 **워커 컨테이너 1개당** 동시 스테이지 실행 수를 제어합니다 (기본 `4`). Compose에서 `worker`를 스케일하면 컨테이너 수 × `WORKER_CONCURRENCY` 만큼 병렬 처리됩니다.

### 프로젝트 생성 (multipart)

| 필드 | 필수 | 설명 |
|------|------|------|
| `plan` | ✅ | PDF 외주 계획서 |
| `devops_plan_text` | — | DevOps 계획서 직접 작성 (Markdown/YAML) |
| `devops_plan` | — | DevOps 계획서 파일 (.md, .yaml, .yml, .txt, .pdf) |
| `name` | — | 프로젝트 이름 |
| `repo_url` | — | GitHub 레포 URL |

## 환경 변수

전체 목록은 [.env.example](.env.example) 참고. 주요 카테고리:

- **서버**: `HOST`, `PORT`, `RUST_LOG`
- **AI API 키**: `OMNIROUTE_API_KEY`/`OMNIROUTE_BASE_URL` (기본 `http://127.0.0.1:20128/v1`), legacy `CURSOR_API_KEY`, `STITCH_API_KEY`, Stitch Bearer (`STITCH_ACCESS_TOKEN` 또는 ADC/gcloud 자동 갱신 — [상세](docs/STITCH_ACCESS_TOKEN.md))
- **모델/예산**: `OMNIROUTE_MODEL_*`, `AI_PROJECT_BUDGET_USD`, `AI_TASK_BUDGET_USD`, bounded plan/debug escalation 설정
- **GitHub 자동화**: `GITHUB_TOKEN`, `GITHUB_ORG`, `GITHUB_AUTO_MERGE`
- **보안**: `API_KEY`, `CORS_ALLOWED_ORIGINS`, `MAX_UPLOAD_BYTES` — 운영 배포 전 반드시 확인
- **아티팩트/이미지 저장소 (로컬 디스크)**: `ARTIFACTS_DIR`, `MAX_IMAGE_BYTES`
- **RabbitMQ (분산 모드)**: `MESSAGE_QUEUE_ENABLED`, `RABBITMQ_URL` 등 — 기본값은 단일 프로세스(false). Redis는 프로젝트 스토어/알림용
- **Slack 알림**: `SLACK_WEBHOOK_URL` 또는 `SLACK_BOT_TOKEN`+`SLACK_CHANNEL`

서버 기동 시 누락되거나 위험한 설정(예: `API_KEY` 또는 `OMNIROUTE_API_KEY` 미설정)은
로그에 경고로 출력됩니다.

## 운영 환경 체크리스트

실사용(프로덕션) 배포 전 최소한 아래 항목을 확인하세요.

- [ ] `CURSOR_API_KEY`, `STITCH_API_KEY`, `STITCH_ACCESS_TOKEN` 설정 — Design 단계에 Stitch Bearer 토큰 필수 ([docs/STITCH_ACCESS_TOKEN.md](docs/STITCH_ACCESS_TOKEN.md))
- [ ] `API_KEY` 설정 — 미설정 시 REST API가 인증 없이 공개됨
- [ ] `CORS_ALLOWED_ORIGINS`를 실제 프론트엔드 도메인으로 제한
- [ ] `ARTIFACTS_DIR`가 영속 볼륨(디스크)을 가리키는지 확인 — Compose 환경에서는
  `artifacts-data` 볼륨이 api/worker/orchestrator 간에 공유되도록 이미 구성되어 있음
- [ ] nginx에 TLS 인증서 적용 (`nginx/nginx.conf`의 HTTPS 서버 블록 주석 참고)
- [ ] 여러 호스트(노드)에 분산 배포하는 경우 `ARTIFACTS_DIR`를 NFS 등 네트워크 파일시스템으로 교체
- [ ] CI(`.github/workflows/ci.yml`)가 통과하는지 확인 (fmt/clippy/test/build + compose smoke test)

## 상세 문서

- [docs/STITCH_ACCESS_TOKEN.md](docs/STITCH_ACCESS_TOKEN.md) — Stitch Bearer 토큰 없을 때 동작
- [docs/PODMAN.md](docs/PODMAN.md) — Podman / Compose 배포
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — 아키텍처 설계
- [docs/QUALITY_WORKFLOW.md](docs/QUALITY_WORKFLOW.md) — 품질 게이트
- [docs/PRODUCTION_READINESS.md](docs/PRODUCTION_READINESS.md) — 실사용 전환을 위해 이번에 개선한 항목과 남은 과제

## 라이선스

Apache-2.0
