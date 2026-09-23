use autoforge::services::usage_ledger::{
    CallDescriptor, RedisUsageLedger, Reservation, UsageLedger,
};
use std::sync::Arc;

const LIMIT_MICRO_USD: u64 = 1_000_000;

fn redis_url() -> Option<String> {
    std::env::var("TEST_REDIS_URL")
        .ok()
        .filter(|value| !value.is_empty())
}

fn call(project: &str, slot: &str) -> CallDescriptor {
    CallDescriptor {
        call_id: CallDescriptor::stable_id(project, "extract", 0, slot),
        project_id: project.into(),
        task_id: None,
        stage: "extract".into(),
        model: "test/model".into(),
        purpose: "extraction".into(),
    }
}

async fn cleanup(url: &str, project: &str) {
    if let Ok(client) = redis::Client::open(url) {
        if let Ok(mut conn) = client.get_connection_manager().await {
            let _: Result<i64, _> = redis::cmd("DEL")
                .arg(format!("autoforge:usage:{project}:records"))
                .arg(format!("autoforge:usage:{project}:total"))
                .query_async(&mut conn)
                .await;
        }
    }
}

#[tokio::test]
async fn concurrent_reservations_replay_and_restart_stay_within_budget() {
    let Some(url) = redis_url() else {
        eprintln!("skipping cost_ledger_redis: TEST_REDIS_URL is not set");
        return;
    };

    let project = format!("test-{}", uuid::Uuid::new_v4());
    let ledger: Arc<dyn UsageLedger> = Arc::new(
        RedisUsageLedger::connect(&url)
            .await
            .expect("connect to test redis"),
    );

    let mut handles = Vec::new();
    for index in 0..8 {
        let ledger = ledger.clone();
        let project = project.clone();
        handles.push(tokio::spawn(async move {
            ledger
                .reserve(
                    &call(&project, &format!("c{index}")),
                    300_000,
                    LIMIT_MICRO_USD,
                    LIMIT_MICRO_USD,
                )
                .await
                .expect("reserve")
        }));
    }

    let mut granted = 0;
    for handle in handles {
        if matches!(handle.await.expect("join"), Reservation::Granted) {
            granted += 1;
        }
    }
    assert_eq!(
        granted, 3,
        "only three 300k reservations fit in a 1M budget"
    );

    let duplicate = ledger
        .reserve(
            &call(&project, "dup"),
            100_000,
            LIMIT_MICRO_USD,
            LIMIT_MICRO_USD,
        )
        .await
        .expect("reserve dup");
    assert_eq!(duplicate, Reservation::Granted);
    let replay = ledger
        .reserve(
            &call(&project, "dup"),
            100_000,
            LIMIT_MICRO_USD,
            LIMIT_MICRO_USD,
        )
        .await
        .expect("replay dup");
    assert!(
        matches!(replay, Reservation::Duplicate(_)),
        "replayed call id must not reserve twice: {replay:?}"
    );

    let restarted: Arc<dyn UsageLedger> = Arc::new(
        RedisUsageLedger::connect(&url)
            .await
            .expect("reconnect after simulated restart"),
    );
    let summary = restarted
        .summary(&project)
        .await
        .expect("summary after restart");
    assert_eq!(summary.records.len(), 4);
    assert!(summary.committed_micro_usd <= LIMIT_MICRO_USD);

    cleanup(&url, &project).await;
}
