use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

pub(crate) mod wire_names;

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
    #[serde(rename = "redis_daily_end_user_spend_update_queue")]
    RedisDailyEndUserQueue,
    RedisDailyOrgSpendUpdateQueue,
    RedisDailyTeamSpendUpdateQueue,
    #[serde(rename = "redis_daily_agent_spend_update_queue")]
    RedisDailyAgentQueue,
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
        Self::RedisDailyEndUserQueue,
        Self::RedisDailyOrgSpendUpdateQueue,
        Self::RedisDailyTeamSpendUpdateQueue,
        Self::RedisDailyAgentQueue,
        Self::RedisDailyTagSpendUpdateQueue,
        Self::InMemorySpendUpdateQueue,
        Self::RedisSpendUpdateQueue,
        Self::RedisWindowSpendUpdateQueue,
    ];

    // PrometheusServicesLogger's gauge services; every other service takes the counter and histogram default
    #[must_use]
    pub const fn metrics(self) -> &'static [ServiceMetric] {
        match self {
            Self::PodLockManager
            | Self::InMemoryDailySpendUpdateQueue
            | Self::RedisDailySpendUpdateQueue
            | Self::RedisDailyEndUserQueue
            | Self::RedisDailyAgentQueue
            | Self::InMemorySpendUpdateQueue
            | Self::RedisSpendUpdateQueue => &[ServiceMetric::Gauge],
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
            | Self::RedisDailyTagSpendUpdateQueue
            | Self::RedisWindowSpendUpdateQueue => {
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

// The model paths a proxy request can take, and the LiteLLM call_type each one reports
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    Chat,
    TextCompletion,
    Embedding,
}

impl ServiceKind {
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        if path.ends_with("/chat/completions") {
            Some(Self::Chat)
        } else if path.ends_with("/completions") {
            Some(Self::TextCompletion)
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
            Self::TextCompletion => "atext_completion",
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
