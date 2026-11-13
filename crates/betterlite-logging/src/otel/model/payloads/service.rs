use super::super::utils::coercion::OtelModelValue;
use super::{error::SpanError, sanitize_event_metadata::sanitize_event_metadata};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceType {
    Redis,
    Postgres,
    BatchWriteToDb,
    ResetBudgetJob,
    SelfService,
    Router,
    Auth,
    ProxyPreCall,
    PodLockManager,
    InMemoryDailySpendUpdateQueue,
    RedisDailySpendUpdateQueue,
    RedisDailyEndUserQueue,
    RedisDailyOrgSpendUpdateQueue,
    RedisDailyTeamSpendUpdateQueue,
    RedisDailyAgentQueue,
    RedisDailyTagSpendUpdateQueue,
    InMemorySpendUpdateQueue,
    RedisSpendUpdateQueue,
    RedisWindowSpendUpdateQueue,
}

impl ServiceType {
    const fn value(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::Postgres => "postgres",
            Self::BatchWriteToDb => "batch_write_to_db",
            Self::ResetBudgetJob => "reset_budget_job",
            Self::SelfService => "self",
            Self::Router => "router",
            Self::Auth => "auth",
            Self::ProxyPreCall => "proxy_pre_call",
            Self::PodLockManager => "pod_lock_manager",
            Self::InMemoryDailySpendUpdateQueue => "in_memory_daily_spend_update_queue",
            Self::RedisDailySpendUpdateQueue => "redis_daily_spend_update_queue",
            Self::RedisDailyEndUserQueue => "redis_daily_end_user_spend_update_queue",
            Self::RedisDailyOrgSpendUpdateQueue => "redis_daily_org_spend_update_queue",
            Self::RedisDailyTeamSpendUpdateQueue => "redis_daily_team_spend_update_queue",
            Self::RedisDailyAgentQueue => "redis_daily_agent_spend_update_queue",
            Self::RedisDailyTagSpendUpdateQueue => "redis_daily_tag_spend_update_queue",
            Self::InMemorySpendUpdateQueue => "in_memory_spend_update_queue",
            Self::RedisSpendUpdateQueue => "redis_spend_update_queue",
            Self::RedisWindowSpendUpdateQueue => "redis_window_spend_update_queue",
        }
    }
}

pub struct ServiceLoggerPayload {
    pub service: ServiceType,
    pub call_type: String,
    pub caller: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceSpanData {
    pub service_name: String,
    pub call_type: Option<String>,
    pub caller: Option<String>,
    pub error: Option<SpanError>,
    pub event_metadata: BTreeMap<String, String>,
}

impl ServiceSpanData {
    #[must_use]
    pub fn from_payload(
        payload: &ServiceLoggerPayload,
        metadata: &BTreeMap<String, OtelModelValue>,
    ) -> Self {
        let error_data = payload.error.clone().map(|message| SpanError {
            message: Some(message),
            error_type: None,
            code: None,
            stack_trace: None,
            llm_provider: None,
        });
        Self {
            service_name: payload.service.value().to_owned(),
            call_type: Some(payload.call_type.clone()),
            caller: payload.caller.clone(),
            error: error_data,
            event_metadata: sanitize_event_metadata(metadata),
        }
    }
}

#[path = "service/tests.rs"]
mod tests;
