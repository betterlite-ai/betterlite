use betterlite_logging::otel::model::payloads::service::{ServiceLoggerPayload, ServiceType};

use super::ServiceInput;
use super::values::helper_error;

pub(super) fn service_type(name: &str) -> Result<ServiceType, Box<crate::helpers::HelperError>> {
    match name {
        "redis" => Ok(ServiceType::Redis),
        "postgres" => Ok(ServiceType::Postgres),
        "batch_write_to_db" => Ok(ServiceType::BatchWriteToDb),
        "reset_budget_job" => Ok(ServiceType::ResetBudgetJob),
        "self" => Ok(ServiceType::SelfService),
        "router" => Ok(ServiceType::Router),
        "auth" => Ok(ServiceType::Auth),
        "proxy_pre_call" => Ok(ServiceType::ProxyPreCall),
        "pod_lock_manager" => Ok(ServiceType::PodLockManager),
        "in_memory_daily_spend_update_queue" => Ok(ServiceType::InMemoryDailySpendUpdateQueue),
        "redis_daily_spend_update_queue" => Ok(ServiceType::RedisDailySpendUpdateQueue),
        "redis_daily_end_user_spend_update_queue" => Ok(ServiceType::RedisDailyEndUserQueue),
        "redis_daily_org_spend_update_queue" => Ok(ServiceType::RedisDailyOrgSpendUpdateQueue),
        "redis_daily_team_spend_update_queue" => Ok(ServiceType::RedisDailyTeamSpendUpdateQueue),
        "redis_daily_agent_spend_update_queue" => Ok(ServiceType::RedisDailyAgentQueue),
        "redis_daily_tag_spend_update_queue" => Ok(ServiceType::RedisDailyTagSpendUpdateQueue),
        "in_memory_spend_update_queue" => Ok(ServiceType::InMemorySpendUpdateQueue),
        "redis_spend_update_queue" => Ok(ServiceType::RedisSpendUpdateQueue),
        "redis_window_spend_update_queue" => Ok(ServiceType::RedisWindowSpendUpdateQueue),
        other => Err(helper_error(
            "InvalidInput",
            format!("unknown service {other}"),
        )),
    }
}

pub(super) fn service_payload(
    input: &ServiceInput,
) -> Result<ServiceLoggerPayload, Box<crate::helpers::HelperError>> {
    Ok(ServiceLoggerPayload {
        service: service_type(&input.service)?,
        call_type: input.call_type.clone(),
        caller: input.caller.clone(),
        error: input.error.clone(),
    })
}
