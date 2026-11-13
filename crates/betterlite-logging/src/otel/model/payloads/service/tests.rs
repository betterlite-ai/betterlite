#![cfg(test)]

use super::*;
#[test]
fn service_payload_sanitizes_metadata() {
    let metadata = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(
        r#"{"token":"secret","zone":"west"}"#,
    )
    .unwrap();
    let payload = ServiceLoggerPayload {
        service: ServiceType::Redis,
        call_type: "get".to_owned(),
        caller: None,
        error: None,
    };
    let data = ServiceSpanData::from_payload(&payload, &metadata);
    assert_eq!(data.event_metadata.get("zone"), Some(&"west".to_owned()));
    assert!(!data.event_metadata.contains_key("token"));
    assert_eq!(data.service_name, "redis");
    assert_eq!(data.call_type.as_deref(), Some("get"));
    assert!(data.error.is_none());
}

#[test]
fn service_carries_caller_and_failure() {
    let payload = ServiceLoggerPayload {
        service: ServiceType::RedisDailyEndUserQueue,
        call_type: "write".to_owned(),
        caller: Some("worker".to_owned()),
        error: Some("failed".to_owned()),
    };
    let data = ServiceSpanData::from_payload(&payload, &BTreeMap::new());
    assert_eq!(data.service_name, "redis_daily_end_user_spend_update_queue");
    assert_eq!(data.caller.as_deref(), Some("worker"));
    assert_eq!(data.error.unwrap().message.as_deref(), Some("failed"));
}

#[rstest::rstest]
#[case::redis(ServiceType::Redis, "redis")]
#[case::postgres(ServiceType::Postgres, "postgres")]
#[case::batch_write(ServiceType::BatchWriteToDb, "batch_write_to_db")]
#[case::reset_budget(ServiceType::ResetBudgetJob, "reset_budget_job")]
#[case::self_service(ServiceType::SelfService, "self")]
#[case::router(ServiceType::Router, "router")]
#[case::auth(ServiceType::Auth, "auth")]
#[case::proxy_pre_call(ServiceType::ProxyPreCall, "proxy_pre_call")]
#[case::pod_lock(ServiceType::PodLockManager, "pod_lock_manager")]
#[case::memory_daily(
    ServiceType::InMemoryDailySpendUpdateQueue,
    "in_memory_daily_spend_update_queue"
)]
#[case::redis_daily(
    ServiceType::RedisDailySpendUpdateQueue,
    "redis_daily_spend_update_queue"
)]
#[case::redis_end_user(
    ServiceType::RedisDailyEndUserQueue,
    "redis_daily_end_user_spend_update_queue"
)]
#[case::redis_org(
    ServiceType::RedisDailyOrgSpendUpdateQueue,
    "redis_daily_org_spend_update_queue"
)]
#[case::redis_team(
    ServiceType::RedisDailyTeamSpendUpdateQueue,
    "redis_daily_team_spend_update_queue"
)]
#[case::redis_agent(
    ServiceType::RedisDailyAgentQueue,
    "redis_daily_agent_spend_update_queue"
)]
#[case::redis_tag(
    ServiceType::RedisDailyTagSpendUpdateQueue,
    "redis_daily_tag_spend_update_queue"
)]
#[case::memory_spend(ServiceType::InMemorySpendUpdateQueue, "in_memory_spend_update_queue")]
#[case::redis_spend(ServiceType::RedisSpendUpdateQueue, "redis_spend_update_queue")]
#[case::redis_window(
    ServiceType::RedisWindowSpendUpdateQueue,
    "redis_window_spend_update_queue"
)]
fn service_names_match_litellm(#[case] service: ServiceType, #[case] expected: &str) {
    let payload = ServiceLoggerPayload {
        service,
        call_type: "operation".to_owned(),
        caller: None,
        error: None,
    };
    let data = ServiceSpanData::from_payload(&payload, &BTreeMap::new());
    assert_eq!(data.service_name, expected);
}
