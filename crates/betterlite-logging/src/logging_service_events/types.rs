use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

pub const CANCELLED_ERROR: &str = "the request was cancelled";

// LiteLLM's ServiceTypes enum; a payload names the service with the same strings
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    Redis,
    #[serde(rename = "postgres")]
    Postgres,
    BatchWriteToDb,
    ResetBudgetJob,
    #[serde(rename = "self")]
    Litellm,
    Router,
    Auth,
    ProxyPreCall,
    PodLockManager,
    InMemoryDailySpendUpdateQueue,
    RedisDailySpendUpdateQueue,
    RedisDailyEndUserSpendUpdateQueue,
    RedisDailyOrgSpendUpdateQueue,
    RedisDailyTeamSpendUpdateQueue,
    RedisDailyAgentSpendUpdateQueue,
    RedisDailyTagSpendUpdateQueue,
    InMemorySpendUpdateQueue,
    RedisSpendUpdateQueue,
    RedisWindowSpendUpdateQueue,
}

impl ServiceType {
    pub const ALL: [Self; 19] = [
        Self::Redis,
        Self::Postgres,
        Self::BatchWriteToDb,
        Self::ResetBudgetJob,
        Self::Litellm,
        Self::Router,
        Self::Auth,
        Self::ProxyPreCall,
        Self::PodLockManager,
        Self::InMemoryDailySpendUpdateQueue,
        Self::RedisDailySpendUpdateQueue,
        Self::RedisDailyEndUserSpendUpdateQueue,
        Self::RedisDailyOrgSpendUpdateQueue,
        Self::RedisDailyTeamSpendUpdateQueue,
        Self::RedisDailyAgentSpendUpdateQueue,
        Self::RedisDailyTagSpendUpdateQueue,
        Self::InMemorySpendUpdateQueue,
        Self::RedisSpendUpdateQueue,
        Self::RedisWindowSpendUpdateQueue,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::Postgres => "postgres",
            Self::BatchWriteToDb => "batch_write_to_db",
            Self::ResetBudgetJob => "reset_budget_job",
            Self::Litellm => "self",
            Self::Router => "router",
            Self::Auth => "auth",
            Self::ProxyPreCall => "proxy_pre_call",
            Self::PodLockManager => "pod_lock_manager",
            Self::InMemoryDailySpendUpdateQueue => "in_memory_daily_spend_update_queue",
            Self::RedisDailySpendUpdateQueue => "redis_daily_spend_update_queue",
            Self::RedisDailyEndUserSpendUpdateQueue => "redis_daily_end_user_spend_update_queue",
            Self::RedisDailyOrgSpendUpdateQueue => "redis_daily_org_spend_update_queue",
            Self::RedisDailyTeamSpendUpdateQueue => "redis_daily_team_spend_update_queue",
            Self::RedisDailyAgentSpendUpdateQueue => "redis_daily_agent_spend_update_queue",
            Self::RedisDailyTagSpendUpdateQueue => "redis_daily_tag_spend_update_queue",
            Self::InMemorySpendUpdateQueue => "in_memory_spend_update_queue",
            Self::RedisSpendUpdateQueue => "redis_spend_update_queue",
            Self::RedisWindowSpendUpdateQueue => "redis_window_spend_update_queue",
        }
    }

    // PrometheusServicesLogger's _get_service_metrics_initialize: the configured gauge services keep
    // their gauge, and every other service takes the counter and histogram default
    #[must_use]
    pub const fn metrics(self) -> &'static [ServiceMetric] {
        match self {
            Self::PodLockManager
            | Self::InMemoryDailySpendUpdateQueue
            | Self::RedisDailySpendUpdateQueue
            | Self::RedisDailyEndUserSpendUpdateQueue
            | Self::RedisDailyAgentSpendUpdateQueue
            | Self::InMemorySpendUpdateQueue
            | Self::RedisSpendUpdateQueue
            | Self::RedisWindowSpendUpdateQueue => &[ServiceMetric::Gauge],
            Self::Redis
            | Self::Postgres
            | Self::BatchWriteToDb
            | Self::ResetBudgetJob
            | Self::Litellm
            | Self::Router
            | Self::Auth
            | Self::ProxyPreCall
            | Self::RedisDailyOrgSpendUpdateQueue
            | Self::RedisDailyTeamSpendUpdateQueue
            | Self::RedisDailyTagSpendUpdateQueue => {
                &[ServiceMetric::Counter, ServiceMetric::Histogram]
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceMetric {
    Counter,
    Histogram,
    Gauge,
}

// LiteLLM's ServiceEventMetadata TypedDict, whose fields a caller sets only when it has them
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceEventMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gauge_labels: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gauge_value: Option<f64>,
}

// LiteLLM's ServiceLoggerPayload, in the field order model_dump writes
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ServiceLoggerPayload {
    pub is_error: bool,
    pub error: Option<String>,
    pub service: ServiceType,
    pub duration: f64,
    pub call_type: String,
    pub caller: Option<String>,
    pub event_metadata: Option<ServiceEventMetadata>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceEvent {
    pub call_id: String,
    pub payload: ServiceLoggerPayload,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

// The two model paths a proxy request can take, and the LiteLLM call_type each one reports
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    Chat,
    Embedding,
}

impl ServiceKind {
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        if path.ends_with("/chat/completions") {
            Some(Self::Chat)
        } else if path.ends_with("/embeddings") {
            Some(Self::Embedding)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn call_type(self) -> &'static str {
        match self {
            Self::Chat => "acompletion",
            Self::Embedding => "aembedding",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalOutcome {
    Succeeded,
    Failed(String),
    Cancelled,
}
