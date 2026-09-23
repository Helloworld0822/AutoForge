# 구조 평가와 V2 후속 계획

## 냉정한 결론

현재 AutoForge는 **OmniRoute 추출·계획·진단 + Cursor 원격 구현**의 하이브리드입니다.
전체 V2 완료나 비용 상한이 보장된 task runner라고 부를 수 없습니다.
이번 변경은 실제 토큰 입력 절감과 provider 교체를 진전시켰지만, 중요한 실행·정산 경계는 아직 남아 있습니다.

유지할 장점: RabbitMQ worker 분리, Redis/Memory store 계약, artifact 중심 교환,
프로젝트별 모델 override, 기존 GitHub delivery와 Q&A gate입니다.
이 계층을 버리고 재작성할 이유는 없습니다.

## 진행 현황 (이번 반복)

branch `capricorn`에서 아래 P0 항목을 구현·테스트·커밋했습니다.

- **P0#1 검증 대상 revision 고정** — 구현 PR의 브랜치/head SHA를 기록하고 verify/debug/security를 PR head에서 실행합니다. SecurityPatch가 head를 이동시키면 재검증을 강제하고, 병합은 검증된 SHA에 대해서만 조건부로 수행합니다. (`434942d`, `91bc4da`)
- **P0#2 실패 폐쇄 게이트** — verify/security 리포트는 엄격 JSON만 허용하며, 파싱 불가·근거 없는 성공은 실패로 처리합니다. SecurityPatch 미통과 시 Deliver와 merge를 모두 차단합니다. (`7e2f2ef`)
- **P0#4 아티팩트 전달 경계** — 허용된 생성 산출물만 30분 만료 서명 토큰으로 `/artifacts/coder`에서 전달하고, 원문 PDF/추출 텍스트/내부 상태는 allowlist로 차단합니다. 토큰 발급이 불가하면 bounded inline으로 폴백하며 자격증명 의심 내용은 제외합니다. (`97ca96a`)

남은 P0#3(원자적 usage 원장·예산 예약)은 Redis 동시성/재시작 통합 테스트와 검증된 가격 catalog가 필요해 별도 반복으로 분리합니다.
또한 revision/security 게이트의 MQ 이벤트 경로와 merge 거부에 대한 end-to-end 통합 테스트는 아직 없습니다. 현재 근거는 단위 테스트입니다.

## 근거 있는 주요 결함

| 우선순위 | 현재 코드 근거 | 영향 / 다음 조치 |
|---|---|---|
| P0 | worker/implementation.rs는 Cursor create_agent 호출 | OMNIROUTE_MODEL_CODE가 coder를 바꾸지 않음. bounded task runner와 executor 연결 필요 |
| P0 | worker.rs의 agent_opts가 pr_url을 무시하고 main 지정 | Verify/Debug가 구현 PR의 동일 commit을 검증한다는 증거 없음. 실행별 ref/SHA 고정 필요 |
| P0 | cost_manager.rs는 호출 경로에 연결되지 않음 | AI_PROJECT_BUDGET_USD/AI_TASK_BUDGET_USD는 실제 지출 차단이 아님. 예약/정산을 provider 앞에 배치 |
| P0 | LocalArtifactStore가 /artifacts URI를 만들지만 현재 routes/nginx에는 해당 서빙 경로가 없음 | URI만 받는 원격 worker가 계획을 읽을 수 없음. 인증된 전달 또는 bounded inline context 필요 |
| P0 | engine.rs의 SecurityPatch 분기가 report.passed와 무관하게 완료 처리 | GitHub merge helper는 false를 막지만 pipeline delivery가 성공처럼 진행될 수 있음. 실패 gate 강화 |
| P1 | ContextManager index/selection은 unit-tested 도구이며 implement에서 호출하지 않음 | 전체 repository 반복 전송을 AutoForge가 통제한다고 주장할 수 없음 |
| P1 | tasks.json DAG는 검증되지만 scheduler는 stage DAG만 실행 | 개별 task 완료/재검증/재시작, session_state/decisions/code_index 영속화 미완성 |
| P1 | stage metadata는 재실행 때 덮어씀; 실패 응답 usage의 영구 원장 없음 | 전체 project 비용/실패 재시도 비용 누락. unknown cost를0으로 처리해서는 안 됨 |
| P1 | native OCR 기능 없음; 긴 입력은 제한 초과 시 실패 | 스캔 문서와 장문 문서는 별도 bounded OCR/chunk extraction 설계 필요 |
| P1 | MQ 중복 전달에 대한 외부 AI 호출 idempotency/reservation 없음 | 재전달 때 중복 과금과 오래된 stage completion 위험 |
| P2 | health/readiness의 OmniRoute 가용성 진단 부족 | /models 빈 목록이 설정 누락과 장애를 구분하지 못함 |
| P2 | 일부 큰 모듈과 문자열 기반 상태/진단 계약 | 모듈별 소유권과 typed boundary 강화, 전체 파일 일괄 재작성은 피함 |

## 이번에 실제 연결한 절감책

- pdf-inspector full-page classification + compact Markdown, 숫자·표·페이지 참조 보존 설정.
- 원문 대신 Markdown 추출, DevOps 요구사항도 추출에 포함.
- 모든 project_spec 필드 존재/타입 확인, 스키마 재호출 최대2회.
- source/model/prompt/output-limit 지문을 사용한 프로젝트별 extraction cache.
- 질문2K·진단4K·추출/계획16K 출력 상한, 입력 상한 초과 명시적 실패.
- compact JSON 계획 입력, UI-only Stitch 입력, 구현 URI allowlist.
- 실제 검증 오류와 직전 debug 요약만 압축 전달, URI 중복 병합.
- schema 변경으로 preferred_stack의 Rust가 TypeScript 기본값으로 사라지던 연결 오류 수정.

고정 절감률은 주장하지 않습니다. mock token counts는 실제 과금 절감 데이터가 아닙니다.
인덱스 선택 테스트가 통과했다는 사실과 실 coder에 연결됐다는 주장은 구분해야 합니다.

## 다음 구현 순서와 완료 기준

1. **안전한 전달·검증 경계부터 수정**
   - PR branch/head SHA를 Project execution state에 저장하고 Verify/Debug/Security에 동일 SHA 전달.
   - SecurityPatch passed=false 또는 checks 비어 있음에서 Deliver/merge 모두 차단.
   - artifact ACL/만료 토큰 또는 bounded inline 전달. raw PDF/secret은 원격 coder 대상 제외.
   - 완료 기준: 서로 다른 main/PR fixture에서 PR만 검증하고, 실패 보고서로 완료/merge가 절대 일어나지 않는 통합 테스트.

2. **project/task usage 원장과 예산 예약**
   - 호출 ID, project/task/stage/model, token/cache/cost, pricing provenance를 append-only 저장.
   - Redis atomic reservation + MemoryStore equivalent. MQ replay는 같은 호출 ID로 dedup.
   - 실제 cost 미제공은 unknown; 검증된 pricing 없으면 고가 escalation 차단, guessed price 금지.
   - 완료 기준: 동시2호출/재전달/timeout/프로세스 재시작에서도 예산 초과 신규 예약0건, 실패 호출 사용량 누락0건.

3. **OmniRoute coder + 격리 executor 연결**
   - 기존 GitHub repo/branch/PR 계층을 재사용하고 checkout/patch/verify만 executor 계약으로 분리.
   - 별도 container/권한/네트워크 정책, 경로·symlink·diff 검증, 명령 allowlist와 timeout.
   - 모델이 제안한 arbitrary shell은 실행하지 않음. npm/cargo scripts 역시 신뢰되지 않는 코드임을 전제로 격리.
   - 완료 기준: mock coder가 unified diff를 출력하고 실제 임시 workspace의 build/test가 실패→수정→성공; path escape·timeout·동일 patch 반복 거부.

4. **task DAG와 ContextManager 결합**
   - task별 checkpoint, dependency 완료 조건, code_index/session_state/decisions 저장.
   - repo 전체 대신 관련2~8파일을 선택하고 token/file omission reason과 source SHA를 기록.
   - 변경 파일만 재인덱싱하고 이미 완료한 task는 재호출하지 않음.
   - 완료 기준: 큰 fixture repo에서도 제한 context, worker 재시작 후 다음 task부터 재개, UI/API-only 양쪽 통합 시나리오 통과.

5. **실측 후 추가 절감과 운영 관측**
   - extraction cache hit율, task당 input/output/cached tokens, retry 낭비, 모델별 실제 비용을 대시보드에 노출.
   - 장문은 source-ref를 보존하는 bounded chunk extraction/병합; 충돌과 누락 검사 없이는 임의 요약 금지.
   - OCR은 필요한 페이지만 별도 격리 worker, native PDF에 외부 OCR 비용 발생 금지.
   - 완료 기준: 고정 corpus에서 요구사항 recall 회귀 없음, 실제 provider usage로 비용 비교, unknown 비용 별도 표시.

이 순서가 필요한 이유는 단순합니다. 지금 모델을 더 추가하면 잘못된 ref 검증과 비용 누락까지 확대됩니다.
먼저 실행 대상과 과금 경계를 확정해야 토큰 절감도 신뢰할 수 있게 측정할 수 있습니다.

## 검증 해석

로컬 단위/통합 테스트와 mock gateway를 사용한 HTTP·브라우저 실행은 수행했습니다.
실제 유료 OmniRoute/Cursor/Stitch/GitHub end-to-end는 키와 실행 환경이 없어 검증하지 않았습니다.
하위 에이전트 사용량 한도로 최종 독립5-lane review는 미승인 상태이며, 이 문서는 주 에이전트의 근거 기반 평가입니다.

참고 계약: [Cursor model 생략 시 기본 모델 선택](https://cursor.com/docs/cloud-agent/api/endpoints),
[pdf-inspector 공식 저장소](https://github.com/firecrawl/pdf-inspector).
