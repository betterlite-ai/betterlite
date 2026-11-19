use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::service_events::async_service_success_hook;
use crate::logging_service_events::types::ServiceType;

use super::support::{
    Intake, build_logger, field, json, last_queued, message, service_payload, text,
};

#[rstest]
#[case::reset_budget_job(ServiceType::ResetBudgetJob, true)]
#[case::redis(ServiceType::Redis, false)]
#[case::postgres(ServiceType::Postgres, false)]
#[case::batch_write_to_db(ServiceType::BatchWriteToDb, false)]
#[case::litellm(ServiceType::Litellm, false)]
#[case::router(ServiceType::Router, false)]
#[case::auth(ServiceType::Auth, false)]
#[case::proxy_pre_call(ServiceType::ProxyPreCall, false)]
#[case::pod_lock_manager(ServiceType::PodLockManager, false)]
#[case::in_memory_daily_spend_update_queue(ServiceType::InMemoryDailySpendUpdateQueue, false)]
#[case::redis_daily_spend_update_queue(ServiceType::RedisDailySpendUpdateQueue, false)]
#[case::redis_daily_end_user_queue(ServiceType::RedisDailyEndUserQueue, false)]
#[case::redis_daily_org_spend_update_queue(ServiceType::RedisDailyOrgSpendUpdateQueue, false)]
#[case::redis_daily_team_spend_update_queue(ServiceType::RedisDailyTeamSpendUpdateQueue, false)]
#[case::redis_daily_agent_queue(ServiceType::RedisDailyAgentQueue, false)]
#[case::redis_daily_tag_spend_update_queue(ServiceType::RedisDailyTagSpendUpdateQueue, false)]
#[case::in_memory_spend_update_queue(ServiceType::InMemorySpendUpdateQueue, false)]
#[case::redis_spend_update_queue(ServiceType::RedisSpendUpdateQueue, false)]
#[case::redis_window_spend_update_queue(ServiceType::RedisWindowSpendUpdateQueue, false)]
fn datadog_service_success_filtering_behavior(#[case] service: ServiceType, #[case] logged: bool) {
    let (logger, transport) = build_logger(&[("DD_BATCH_SIZE", "1")], Intake::Accepted);
    let payload = service_payload(service, false, None, 0.5);
    async_service_success_hook(&logger, &payload, None);

    // no event flushes, even with a batch size of one
    assert_eq!(transport.post_count(), 0);
    if !logged {
        // a service type the list does not hold returns silently
        assert_eq!(logger.queue.pending().len(), 0);
        return;
    }
    assert_eq!(logger.queue.pending().len(), 1);
    let queued = last_queued(&logger);
    assert_eq!(queued.status, DataDogStatus::Info);
    assert_eq!(queued.trace_id, None);
    assert_eq!(queued.span_id, None);
    let logged_body = message(&queued);
    assert_eq!(field(&logged_body, "is_error"), &Json::Bool(false));
    assert_eq!(field(&logged_body, "error"), &Json::Null);
    assert_eq!(text(field(&logged_body, "service")), "reset_budget_job");
    assert_eq!(field(&logged_body, "duration"), &json("0.5"));
}
