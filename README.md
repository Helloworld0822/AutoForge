# AutoForge — AI 외주 자동화 프로그램

PDF 계획서를 업로드하면 **요약(Sonnet) → 기획(Fable) → 디자인(Stitch) → 구현(Codex 5.3)** 파이프라인이 자동 실행됩니다.

## 프로젝트 구조

```
backend/          # Rust API (Actix-web)
frontend/         # React + Vite + TypeScript UI
nginx/            # Nginx 리버스 프록시 설정
docker-compose.yml
podman-compose.yml
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
                      redis + minio
```

- **nginx**: 프론트엔드 정적 파일 서빙 + API 프록시
- **api**: Rust REST API (프론트엔드 정적 파일 미포함)
- **orchestrator / worker**: Redis Streams 기반 분산 파이프라인

## 빠른 시작 (Docker Compose)

```bash
cp .env.example .env
# .env 편집 (CURSOR_API_KEY, GITHUB_TOKEN 등)

./scripts/compose-up.sh
# 또는
docker compose up -d --build --scale worker=3

open http://localhost
```

| URL | 설명 |
|-----|------|
| http://localhost | React 대시보드 (nginx) |
| http://localhost/v1/projects | API |
| http://localhost:9001 | MinIO 콘솔 |

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

| Method | Path | 설명 |
|--------|------|------|
| GET | `/health` | 헬스체크 |
| POST | `/v1/projects` | PDF 업로드 + DevOps 계획서(선택) + 파이프라인 시작 |
| GET | `/v1/projects` | 프로젝트 목록 |
| GET | `/v1/projects/{id}` | 프로젝트 상세 |
| GET | `/v1/projects/{id}/stream` | SSE 진행률 |
| GET | `/v1/projects/{id}/daily-logs` | 일별 경과 목록 |
| GET | `/v1/projects/{id}/daily-logs/{date}` | 특정 날짜 MD 로그 |

Compose 환경에서는 nginx가 `/v1`, `/health`를 `api:8080`으로 프록시합니다.

### 프로젝트 생성 (multipart)

| 필드 | 필수 | 설명 |
|------|------|------|
| `plan` | ✅ | PDF 외주 계획서 |
| `devops_plan_text` | — | DevOps 계획서 직접 작성 (Markdown/YAML) |
| `devops_plan` | — | DevOps 계획서 파일 (.md, .yaml, .yml, .txt, .pdf) |
| `name` | — | 프로젝트 이름 |
| `repo_url` | — | GitHub 레포 URL |

## 환경 변수

전체 목록은 [.env.example](.env.example) 참고.

## 상세 문서

- [docs/PODMAN.md](docs/PODMAN.md) — Podman / Compose 배포
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — 아키텍처 설계
- [docs/QUALITY_WORKFLOW.md](docs/QUALITY_WORKFLOW.md) — 품질 게이트

## 라이선스

Apache-2.0
