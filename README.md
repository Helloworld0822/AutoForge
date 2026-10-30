# AutoForge — AI 외주 자동화 프로그램

PDF 계획서를 업로드하면 **요약(Haiku) → 기획(Sonnet) → 디자인(Stitch) → 구현(Codex 5.3)** 파이프라인이 자동 실행됩니다.

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
# .env 편집 (CURSOR_API_KEY, GITHUB_TOKEN 등)

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
cd backend && cargo run

# 터미널 2 — 프론트엔드 (API :8080 프록시)
cd frontend && npm install && npm run dev
# http://localhost:5173
```

## GitHub 자동화

`GITHUB_TOKEN` 설정 시 프라이빗 레포 자동 생성 → Cursor PR 생성 → SecurityPatch 통과 후 자동 merge.

```bash
export GITHUB_TOKEN=ghp_xxxx
export GITHUB_ORG=my-org        # 선택
export GITHUB_AUTO_MERGE=true   # 기본값
```

## API 엔드포인트

### 견적과 코딩 스킬

대시보드의 **견적 작성**에서 PDF 계획서나 파이프라인 실행 없이 견적을 작성할 수 있습니다.
요구사항과 1일 단가를 입력하고 **AI 가견적 생성**을 누르면 Cursor의 기본 요약 모델이
작업별 예상 인일과 가정·제외사항을 제안합니다. AI 가견적에는 `CURSOR_API_KEY`가 필요합니다.
생성은 비동기로 진행되며 최대 15분간 결과를 조회합니다. 생성된 항목과 단가를 수정한 뒤
저장하거나, **가견적서 PDF / 견적서 PDF**를 눌러 브라우저 인쇄 창에서 **PDF로 저장**을 선택하세요.
수량은 정수(최대 10,000), 단가는 원 단위(최대 10억 원)이며 합계는 최대 1조 원입니다.
부가세 가산 시 공급가액 합계의 10%를 계산하고 원 미만은 버립니다.
견적·요구사항·단가는 `ARTIFACTS_DIR/estimates/`에 저장됩니다. 컨테이너에서는 기존 산출물 볼륨을 유지해야 합니다.

새 프로젝트에서 **코딩 스킬 · 토큰 절약**을 선택하거나, 프로젝트 상세 화면에서 실행 중에도
변경할 수 있습니다. 관련 코드만 읽기, 최소 변경, 출력 요약 지침과 사용자 Markdown 지침
(직접 입력 또는 `SKILL.md`, 최대 4,000자)을 지원합니다. 선택한 지침만 Implement, Verify,
Debug, Security 단계의 프롬프트에 포함합니다. 실행 중 변경은 **다음 단계부터** 적용됩니다.
Markdown에 연결된 스크립트·외부 스킬을 설치하지 않으며 실제 토큰 절감량을 보장하지 않습니다.
실행 중 설정은 프로젝트 상태와 별도 파일에 저장하므로 파이프라인 결과 저장으로 덮어쓰지 않습니다.

| Method | 경로 | 설명 |
|--------|------|------|
| GET / POST | `/v1/estimates` | 견적 목록 / 신규 저장 |
| GET / PUT | `/v1/estimates/{id}` | 견적 조회 / 수정 |
| POST | `/v1/estimates/generate` | `{title, requirements, daily_rate}`로 AI 가견적 요청 |
| GET | `/v1/estimates/generations/{id}` | 생성 상태와 편집 가능한 가견적 조회 |
| GET / PUT | `/v1/projects/{id}/skills` | `{enabled, custom_markdown}` 스킬 조회 / 적용 |

스킬 ID: `targeted_read`, `minimal_diff`, `concise_output`.
프로젝트 생성 시 `model_config.coding_skills`로도 지정할 수 있으며, 이후 저장한 상세 화면 설정이 우선합니다.


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
- **AI API 키**: `CURSOR_API_KEY`, `STITCH_API_KEY`, Stitch Bearer (`STITCH_ACCESS_TOKEN` 또는 ADC/gcloud 자동 갱신 — [상세](docs/STITCH_ACCESS_TOKEN.md))
- **GitHub 자동화**: `GITHUB_TOKEN`, `GITHUB_ORG`, `GITHUB_AUTO_MERGE`
- **보안**: `API_KEY`, `CORS_ALLOWED_ORIGINS`, `MAX_UPLOAD_BYTES` — 운영 배포 전 반드시 확인
- **아티팩트/이미지 저장소 (로컬 디스크)**: `ARTIFACTS_DIR`, `MAX_IMAGE_BYTES`
- **RabbitMQ (분산 모드)**: `MESSAGE_QUEUE_ENABLED`, `RABBITMQ_URL` 등 — 기본값은 단일 프로세스(false). Redis는 프로젝트 스토어/알림용
- **Slack 알림**: `SLACK_WEBHOOK_URL` 또는 `SLACK_BOT_TOKEN`+`SLACK_CHANNEL`

서버 기동 시 누락되거나 위험한 설정(예: `API_KEY` 미설정, `CURSOR_API_KEY` 비어있음)은
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
