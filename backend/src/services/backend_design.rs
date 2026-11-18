//! Project-specific design decisions and the published case studies that informed them.
use crate::domain::StageId;
use crate::error::{AutoForgeError, Result};
use crate::services::architecture_qa::extract_json_object;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize)]
struct CaseStudy {
    id: &'static str,
    published: &'static str,
    source: &'static str,
    scale: &'static str,
    technique: &'static str,
}

const CASES: &[CaseStudy] = &[
    CaseStudy {
        id: "discord",
        published: "2023-03-06",
        source: "https://discord.com/blog/how-discord-stores-trillions-of-messages",
        scale: "2022년 초 메시지 수조 건을 Cassandra 177개 노드에 저장",
        technique: "API와 DB 사이에서 같은 행에 대한 동시 조회를 병합하고 채널별 라우팅으로 핫 파티션의 부하를 완화",
    },
    CaseStudy {
        id: "slack",
        published: "2023-09-28",
        source: "https://slack.engineering/executing-cron-scripts-reliably-at-scale/",
        scale: "공용 작업 큐에서 하루 약 90억 건의 작업 처리",
        technique: "스케줄링과 워커 실행을 분리하고 기존 Kafka·Redis 작업 큐와 Vitess 중복 제거 테이블을 재사용",
    },
    CaseStudy {
        id: "plausible",
        published: "2022-06-22",
        source: "https://plausible.io/blog/open-source-saas",
        scale: "5만 개 이상 사이트에서 월 10억 페이지뷰 초과",
        technique: "대형 사이트의 통계 대시보드가 느려지자 분석 데이터를 PostgreSQL에서 열 지향 ClickHouse로 이전",
    },
];

pub(super) fn design_instructions() -> String {
    let catalog = serde_json::to_string(CASES).expect("case studies contain only strings");
    format!(
        r#"설계는 strict JSON 객체 하나로 반환하세요. 아래는 필드 형식입니다.
{{"spec_md":"상세 명세, 시스템 아키텍처 설명, 구현 태스크 DAG를 담은 Markdown","backend":{{
  "workload":"프로젝트의 사용자·피크 요청량·데이터량·예산 제약. 수치에 단위와 근거를 붙이고 미상 값은 명시",
  "components":{{"api":{{"role":"역할 및 기술 선택","capacity":"초기 용량과 산정 근거","depends_on":[]}}}},
  "indexes":[{{"component":"저장소 컴포넌트 키","query":"필터·조인·정렬 또는 키 조회 패턴","strategy":"키·컬럼 순서와 인덱스 종류 또는 기존 기본키 활용","validation":"실행 계획이나 명령 수, 쓰기 비용 검증"}}],
  "comparisons":{{"사례 ID":{{"component":"관련 컴포넌트 키","decision":"adopt 또는 defer","reason":"프로젝트 부하와 사례 규모의 차이, 비용과 절충점","validation":"측정할 지표와 도입 또는 재검토 조건"}}}}
}}}}
프로젝트 요약과 답변에 맞춰 컴포넌트와 연결을 설계하세요. 아래 공식 사례 중 관련 사례를 둘 이상 비교하세요.
저장소별 주요 조회의 인덱스를 검토하고 중복 인덱스는 피하세요. 영속 데이터 조회가 없으면 indexes는 빈 배열로 두세요.
공개 당시의 관측치와 프로젝트 목표를 구분하고, 근거 없는 수치는 가정으로 표시하세요.
작은 서비스에서는 단순한 초기 구성을 우선하고, 모든 기법을 보류해도 됩니다.
사례 목록: {catalog}"#
    )
}

#[derive(Deserialize)]
struct ArchitectResponse {
    spec_md: String,
    backend: BackendDesign,
}

#[derive(Deserialize, Serialize)]
struct BackendDesign {
    workload: String,
    components: BTreeMap<String, Component>,
    indexes: Vec<IndexPlan>,
    comparisons: BTreeMap<String, Comparison>,
}

#[derive(Deserialize, Serialize)]
struct Component {
    role: String,
    capacity: String,
    depends_on: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct IndexPlan {
    component: String,
    query: String,
    strategy: String,
    validation: String,
}

#[derive(Deserialize, Serialize)]
struct Comparison {
    component: String,
    decision: Decision,
    reason: String,
    validation: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Decision {
    Adopt,
    Defer,
}

fn invalid(message: impl ToString) -> AutoForgeError {
    AutoForgeError::StageFailed {
        stage: StageId::Architect,
        message: format!("invalid backend design: {}", message.to_string()),
    }
}

pub(super) fn render_response(text: &str) -> Result<(String, String)> {
    let json = extract_json_object(text).ok_or_else(|| invalid("expected JSON object"))?;
    let ArchitectResponse { spec_md, backend } = serde_json::from_value(json).map_err(invalid)?;
    let components = &backend.components;
    if spec_md.trim().is_empty() || backend.workload.trim().is_empty() || components.is_empty() {
        return Err(invalid("spec_md, workload and components are required"));
    }
    for (name, part) in components {
        if name.trim().is_empty()
            || part.role.trim().is_empty()
            || part.capacity.trim().is_empty()
            || part
                .depends_on
                .iter()
                .any(|id| id == name || !components.contains_key(id))
        {
            return Err(invalid(format!(
                "invalid component or dependency in {name}"
            )));
        }
    }
    for index in &backend.indexes {
        if !components.contains_key(&index.component)
            || index.query.trim().is_empty()
            || index.strategy.trim().is_empty()
            || index.validation.trim().is_empty()
        {
            return Err(invalid(
                "invalid index target or missing query, strategy or validation",
            ));
        }
    }
    if backend.comparisons.len() < 2 {
        return Err(invalid("compare at least two distinct case studies"));
    }
    let mut references = Vec::new();
    for (id, comparison) in &backend.comparisons {
        let case = CASES
            .iter()
            .find(|case| case.id == id)
            .ok_or_else(|| invalid(format!("unknown case study {id}")))?;
        if !components.contains_key(&comparison.component)
            || comparison.reason.trim().is_empty()
            || comparison.validation.trim().is_empty()
        {
            return Err(invalid(format!("incomplete comparison for {id}")));
        }
        references.push(case);
    }

    #[derive(Serialize)]
    struct Document<'a> {
        backend: BackendDesign,
        references: Vec<&'a CaseStudy>,
    }
    let hcl = hcl::to_string(&Document {
        backend,
        references,
    })
    .map_err(invalid)?;
    Ok((spec_md, hcl))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn response() -> Value {
        let decision = json!({"component":"api","decision":"defer","reason":"초기에는 단일 인스턴스","validation":"대기 시간 p95 측정"});
        json!({"spec_md":"# 명세","backend":{
            "workload":"피크 20 RPS (계획서)",
            "components":{"api":{"role":"read \"한글\" ${tenant}","capacity":"1 replica","depends_on":["db"]},
                          "db":{"role":"storage","capacity":"10 GB","depends_on":[]}},
            "indexes":[{"component":"db","query":"tenant_id와 created_at 순서로 읽기","strategy":"(tenant_id, created_at)","validation":"EXPLAIN ANALYZE 및 쓰기 지연 측정"}],
            "comparisons":{"discord":decision,"slack":decision}
        }})
    }

    #[test]
    fn design_round_trips_with_literal_text_and_canonical_evidence() {
        let input = response();
        let (spec, hcl) = render_response(&format!("```json\n{input}\n```")).unwrap();
        let parsed: Value = hcl::from_str(&hcl).unwrap();
        assert_eq!(spec, "# 명세");
        assert_eq!(parsed["backend"], input["backend"]);
        assert_eq!(parsed["references"][0]["source"], CASES[0].source);
        assert_eq!(parsed["references"].as_array().unwrap().len(), 2);
        assert!(hcl.contains("$${tenant}"));
    }

    #[test]
    fn rejects_unusable_designs_before_publication() {
        assert!(render_response("plain Markdown").is_err());
        for (path, bad) in [
            ("/spec_md", json!("")),
            ("/backend/comparisons", json!({})),
            ("/backend/components/api/depends_on", json!(["absent"])),
            ("/backend/comparisons/slack/component", json!("missing")),
            ("/backend/comparisons/slack/decision", json!("maybe")),
        ] {
            let mut data = response();
            *data.pointer_mut(path).unwrap() = bad;
            assert!(render_response(&data.to_string()).is_err(), "{path}");
        }
        let mut data = response();
        data["backend"]["comparisons"]["invented"] =
            data["backend"]["comparisons"]["slack"].clone();
        assert!(render_response(&data.to_string()).is_err());
    }

    #[actix_web::test]
    async fn architect_publishes_downloadable_design_and_passes_it_to_implement() {
        use crate::clients::cursor::CursorClient;
        use crate::services::pipeline::engine::{apply_stage_output, execute_stage};
        use actix_web::{test, web, App as WebApp, HttpResponse, HttpServer};
        use std::sync::{Arc, Mutex};

        type Prompts = Arc<Mutex<Vec<String>>>;
        async fn create(body: web::Json<Value>, seen: web::Data<Prompts>) -> HttpResponse {
            let mut prompts = seen.lock().unwrap();
            prompts.push(body["prompt"]["text"].as_str().unwrap().to_owned());
            HttpResponse::Ok().json(json!({
                "agent": {"id": "agent", "status": "RUNNING"},
                "run": {"id": prompts.len().to_string(), "status": "RUNNING"}
            }))
        }
        async fn result(id: web::Path<u8>) -> HttpResponse {
            let text = match *id {
                1 => json!({"workload":"peak 20 RPS"}).to_string(),
                2 => json!({"questions":[]}).to_string(),
                3 => response().to_string(),
                _ => "done".into(),
            };
            HttpResponse::Ok()
                .json(json!({"id":id.to_string(), "status":"COMPLETED", "result":text}))
        }

        let prompts = Prompts::default();
        let received = prompts.clone();
        let server = HttpServer::new(move || {
            WebApp::new()
                .app_data(web::Data::new(received.clone()))
                .route("/v1/agents", web::post().to(create))
                .route("/v1/agents/agent/runs/{id}", web::get().to(result))
        })
        .workers(1)
        .bind(("127.0.0.1", 0))
        .unwrap();
        let base = format!("http://{}", server.addrs()[0]);
        let server = server.run();
        let handle = server.handle();
        actix_web::rt::spawn(server);

        let mut config = crate::Config::from_env();
        config.artifacts_dir = std::env::temp_dir()
            .join(uuid::Uuid::new_v4().to_string())
            .display()
            .to_string();
        config.api_key = Some("test-key".into());
        config.login_password = None;
        let mut app = crate::App::new(config).await.unwrap();
        app.cursor = Arc::new(CursorClient::for_test_server(base));
        let app = Arc::new(app);
        let mut project = app
            .create_project(
                None,
                Some("https://example.com/orders".into()),
                None,
                Default::default(),
                Default::default(),
            )
            .await;
        let plan = app
            .artifacts
            .put(
                &format!("projects/{}/ingest/raw_text.md", project.id.0),
                bytes::Bytes::from_static(b"design a store for 20 requests per second"),
                "text/markdown",
            )
            .await
            .unwrap();
        project.accumulated_artifacts.push(plan);
        let summary = execute_stage(&app, &project, StageId::Summarize)
            .await
            .unwrap();
        apply_stage_output(&mut project, StageId::Summarize, summary).unwrap();
        let design = execute_stage(&app, &project, StageId::Architect)
            .await
            .unwrap();
        assert_eq!(design.artifacts.len(), 2);
        let key = design.artifacts[1].key.clone();
        apply_stage_output(&mut project, StageId::Architect, design).unwrap();
        app.store.save(&project).await.unwrap();
        execute_stage(&app, &project, StageId::Implement)
            .await
            .unwrap();

        let hcl = app.artifacts.get(&key).await.unwrap();
        {
            let seen = prompts.lock().unwrap();
            assert_eq!(seen.len(), 4);
            assert!(seen[0].contains("20 requests per second"));
            assert!(seen[1].contains("peak 20 RPS") && seen[2].contains("peak 20 RPS"));
            assert!(seen[2].contains(CASES[0].source) && seen[2].contains(CASES[1].source));
            assert!(seen[3].contains(std::str::from_utf8(&hcl).unwrap()));
            assert!(seen[3].contains("# 명세"));
        }
        let api = test::init_service(
            WebApp::new()
                .app_data(web::Data::new(app.clone()))
                .configure(crate::web::configure),
        )
        .await;
        let path = format!("/v1/projects/{}/backend.hcl", project.id.0);
        let request = test::TestRequest::get().uri(&path);
        assert_eq!(
            test::call_service(&api, request.to_request())
                .await
                .status(),
            401
        );
        let request = test::TestRequest::get()
            .uri(&path)
            .insert_header(("Authorization", "Bearer test-key"));
        let downloaded = test::call_and_read_body(&api, request.to_request()).await;
        assert_eq!(downloaded, hcl);
        handle.stop(true).await;
        tokio::fs::remove_dir_all(app.media.base_dir())
            .await
            .unwrap();
    }
}
