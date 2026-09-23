# AutoForge 현재 아키텍처

이 문서는 현재 실행 코드를 설명합니다. 최초 V2 목표 전체가 구현된 상태는 아닙니다.
미연결 기능과 후속 우선순위는 [구조 평가](STRUCTURE_REVIEW.md)를 참고하세요.

## 실행 구조

- Rust/Actix API, React/Vite 대시보드.
- 단일 프로세스: MemoryStore + inline scheduler.
- Compose: Redis ProjectStore/pub-sub, RabbitMQ command/event 큐, stateless worker.
- artifact는 로컬 파일이며 api/orchestrator/worker가 artifacts-data 볼륨을 공유합니다. PostgreSQL, Redis Streams 큐, S3/MinIO는 현재 사용하지 않습니다.
- project Git 동기화, GitHub repo/PR/CI/merge, Slack 알림은 기존 계층을 유지합니다.

## 실제 단계와 provider

| 저장/API 단계 | 실행 주체 | 입력 → 출력 |
|---|---|---|
| ingest | 로컬 pdf-inspector, native pdftotext fallback | plan.pdf → raw_text.md, ingest_meta.json |
| extract | OmniRoute extract | Markdown + DevOps text → project_spec.json, extract_cache.json |
| architect | OmniRoute plan / conditional plan_escalation | compact project spec + 사용자 답변 → architecture.md, spec.md, tasks.json, planning_meta.json |
| design | Stitch 또는 기존 Figma 경로 | UI 요구사항 → 화면 참고자료 |
| implement | Cursor Cloud Agent | 허용된 계획/디자인 artifact URI → PR metadata |
| verify | Cursor Cloud Agent | 저장소 검증 지시 → verify_report.json |
| debug | Cursor patch + 필요 시 OmniRoute diagnosis | 압축한 실제 verify_report + 직전 수정 요약 → debug_report.json |
| security_patch | Cursor Cloud Agent | 보안 검사/패치 → security_report.json |
| deliver | 로컬 manifest | 산출물과 stage metadata → delivery_manifest.json |

extract 이후 architect와 design이 실행 가능해집니다. UI 요구사항이 없으면 design은 skipped입니다.
architect는 질문이 있으면 awaiting_input으로 정지하고 답변 후 finalize합니다.
implement는 architect finalize와 design 완료/skip을 기다립니다.
verify 실패 후 debug→verify가 반복되며 상한을 넘으면 failed입니다.
기존 Rust StageId::Summarize 식별자는 유지하되 직렬화 이름은 extract이며 legacy summarize alias를 읽습니다.
독립적인 context/indexing stage나 task 단위 실행 상태 머신은 아직 없습니다.

## OmniRoute 경계

clients/omniroute.rs의 AiProvider 계약과 재사용 reqwest Client를 사용합니다.
기본 주소는 http://127.0.0.1:20128/v1 입니다. OpenRouter로 자동 fallback하지 않습니다.
역할 모델은 OMNIROUTE_MODEL_* 또는 프로젝트 summarize/architect override로 지정합니다.
모델 ID는 인스턴스의 GET /v1/models에서 확인해야 하며 코드에는 추측한 기본 ID가 없습니다.

지원: system/user message, JSON response_format, temperature, max_tokens, token/cache/cost parsing.
응답 본문은 1MiB로 제한하며 빈 응답·잘린 응답·잘못된 JSON을 거부합니다.
429/5xx만 최대3회 재시도합니다. Retry-After 초/HTTP-date를 존중하고30초 초과 대기 요구는 명시적 오류로 반환합니다.
POST의 모호한 네트워크 실패나 timeout은 중복 과금 방지를 위해 자동 재전송하지 않습니다.
HTTP 요청 timeout은120초이며, retry 전체 시간이120초로 제한되는 것은 아닙니다.
모델 목록은5초 timeout입니다. API key나 upstream error body는 로그/오류에 포함하지 않습니다.

GET /v1/models 응답은 models(Cursor)와 omniroute_models를 분리합니다.
현재 gateway 목록 조회 실패는 빈 목록으로 반환하므로 readiness 보장이 아닙니다.
Cursor 모델을 지정하지 않으면 공식 API의 계정/팀/시스템 기본 모델 선택을 사용합니다.

## PDF와 정보 보존

pdf-inspector1.23.0의 full-page detection과 compact Markdown을 사용합니다.
제목, 표, 숫자와 페이지 참조를 보존하도록 설정합니다. 원본 PDF도 artifact에 남깁니다.
불필요한 dot leader와 반복 header/footer 정리는 parser가 수행하며, 추가로 연속 동일 일반 문단을 제거합니다.
표/목록/링크/코드 구문은 일반 문단 중복 제거 대상이 아닙니다.
native encoding 문제 또는 빈 native Markdown에만 pdftotext를 사용합니다.
fallback은 별도 임시 디렉터리와20초 timeout을 사용하며 stdout pipe 교착을 피합니다.
PDF 처리는 spawn_blocking으로 async worker를 막지 않도록 합니다.

OCR 엔진은 포함하지 않습니다. 스캔/혼합 PDF의 미복구 페이지가 있으면 오류로 중단합니다.
OCR 없는 부분 텍스트를 완전한 요구사항으로 가장하지 않습니다.
ingest_meta에는 종류·confidence·페이지 수·OCR 대상·추출 방식·SHA256·토큰 추정치가 기록됩니다.

## 토큰 절약이 실제 적용되는 위치

1. 원문 PDF는 ingest만 읽고 extract에는 Markdown만 전송합니다.
2. extract는 전체 필드와 타입을 확인하며 스키마 오류 시 총2회까지 호출합니다.
3. 캐시 키는 원문·모델·프롬프트 버전·출력 상한을 포함합니다. 같은 프로젝트의 성공한 추출 artifact만 재사용합니다.
4. 계획 요청에는 compact JSON과 필요한 답변만 넣고 전체 대화 기록은 누적하지 않습니다.
5. planner는 정상 계획에서 상위 모델을 호출하지 않습니다. 모순/extreme/confidence<0.7/검증 실패 시만 최대1회 escalation합니다.
6. Stitch 입력은 ui_requirements만 포함합니다.
7. 구현 프롬프트에서는 PDF/raw_text/내부 cache URI를 제외합니다. 단, Cursor 내부의 repository context와 inference는 AutoForge가 제어하지 못합니다.
8. 진단에는 실제 검증 로그와 직전 debug report만 압축해 넣습니다. 진단 evidence JSON은8KiB 상한입니다.
9. 산출물 목록은 key별 최신 참조로 병합하여 재시작 때 같은 URI가 계속 늘어나지 않습니다.

AI_MAX_INPUT_TOKENS 기본30000은 byte 기반 보수적 추정값이며 실제 모델 tokenizer 수치가 아닙니다.
초과 입력은 명시적으로 차단하며 자동 요약/절단하지 않습니다.
출력 상한: extract16000 / plan16000 / clarify2000 / diagnosis4000.
실제 사용량은 provider response의 usage로 stage metadata에 기록합니다. provider가 cost를 생략하면 null입니다.
영구 project/task 통합 원장과 비용 예약은 아직 연결되지 않았으므로 AI_*_BUDGET_USD만으로 비용 상한을 보장할 수 없습니다.

## 아직 기반 도구인 기능

ContextManager는 파일/바이트/개수 제한의 local index, symbol 기반 파일 선택, omission reason, 오류 압축을 제공합니다.
오류 압축은 debug worker에 연결되어 있습니다.
code index와 파일 선택은 Cursor 구현 worker에 연결되어 있지 않으며 session state는 아직 타입 수준입니다.
CostManager 역시 분산 예산 예약/정산 경계가 아닙니다.
OMNIROUTE_MODEL_CODE와 DEBUG_ALT는 현재 전체 coding 실행을 전환하는 스위치가 아닙니다.

## 검증 및 운영

backend의 cargo fmt/check/test/clippy와 frontend의 npm build/lint를 사용합니다.
CI와 Containerfile은 pdftotext fallback을 위해 poppler-utils를 설치합니다.
Rust 최소 버전은 잠금 의존성에 맞춘1.89입니다.
Compose에서 호스트 gateway는 host.containers.internal:20128/v1을 사용합니다.
LSP 미설치/timeout과 컴파일 결과는 별개입니다. 검증 결과는 [품질 문서](QUALITY_WORKFLOW.md)를 참고하세요.
