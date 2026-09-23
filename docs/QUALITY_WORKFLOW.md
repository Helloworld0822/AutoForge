# 품질 워크플로우와 검증 한계

## 현재 실행

implement → verify → (실패 시 debug → verify) → security_patch → deliver.
구현·검증·패치는 현재 Cursor Cloud Agent이며, 로컬 sandbox executor가 아닙니다.
실제 build/test exit code를 AutoForge 자체 프로세스가 수집하는 V2 verifier는 아직 미구현입니다.
VerifyReport의 passed는 원격 agent가 작성한 보고서입니다.

debug attempt는0부터 시작합니다. 기본 설정에서0/1은 Cursor self-fix,
2는 OmniRoute debug 진단 + Cursor patch,3은 debug_escalation 진단 + Cursor patch입니다.
각 debug 뒤 scheduler가 verify를 다시 실행합니다.
상한은 self2, mid1, final1이며 MAX_DEBUG_CYCLES 기본4로 전체 반복도 제한합니다.
설정을0으로 두면 해당 진단 경로가 차단됩니다.
아직 self-fix가 OmniRoute DeepSeek 호출인 것은 아닙니다.

debug는 verify_report.json의 실제 errors와 실패 checks를 읽습니다.
누락/잘못된 보고서는 실패시키며 error count만으로 진단하지 않습니다.
중복 로그와 긴 메시지를 압축하고 직전 debug 요약만 포함합니다.
진단 JSON에는 root_cause, affected_files, recommended_fix, risk, additional_tests가 필요합니다.

SecurityPatch 이후 GitHub 자동 merge는 services/github.rs에서 passed를 확인합니다.
다만 scheduler의 security 완료 처리와 verify의 PR ref 고정에는 남은 결함이 있으므로
운영 자동 merge를 켜기 전에 [구조 평가 P0 항목](STRUCTURE_REVIEW.md)을 해결해야 합니다.

## 검증 명령

```bash
cd backend
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings

cd ../frontend
npm ci
npm run lint
npm run build

cd ..
docker compose config --quiet
```

디스크가 작으면 CARGO_INCREMENTAL=0을 사용하세요. 빌드 캐시를 source commit에 포함하지 않습니다.
pdftotext 회귀 테스트에는 poppler-utils가 필요합니다.

## 이번 검증 범위

- Mock HTTP: chat/models wire contract, auth, usage, invalid/blank/truncated JSON,429/5xx/retry-after, timeout, response size, 오류 secret 비노출.
- Native PDF fixture: compact Markdown, 숫자/표/목록 보존, OCR 필요 페이지 거부, pdftotext fallback.
- Context: relevant ranking, symlink/traversal 거부, 파일/byte 상한, omission reason, UTF-8 error compression.
- Pipeline: schema validation retry, gateway 종료 후 extraction cache 재사용, artifact 중복 방지, UI skip, planner DAG 검증.
- 실제 로컬 API: multipart PDF 업로드→Markdown→mock gateway 추출→설계 질문 대기, 잘못된 PDF400.
- Chromium:1440px/390px, provider 목록 분리, stale ID 표시/제출, empty/error catalog, 모바일 메뉴.

유료 OmniRoute 인스턴스, Cursor 원격 빌드/PR, Stitch 생성, 실제 GitHub merge는 이 환경에서 검증하지 않았습니다.
Mock 사용량은 과금 실측이 아닙니다. 전체 V2 완료 또는 운영 준비 완료로 해석하지 마세요.
