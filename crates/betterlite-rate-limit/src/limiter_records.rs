use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use serde::{Deserialize, Serialize};

mod tests;

pub const PROJECT_ITPM_DESCRIPTOR_KEY: &str = "model_per_project_itpm";
pub const PROJECT_OTPM_DESCRIPTOR_KEY: &str = "model_per_project_otpm";
pub const PARALLEL_SLOT_TTL_SECONDS: i64 = 3600;
pub const DEFAULT_MAX_TOKENS_ESTIMATE: i64 = 4096;
pub const DEFAULT_CHARS_PER_TOKEN: i64 = 4;
pub const TPM_FLOOR_FRACTION: i64 = 4;
pub const RESPONSES_MIN_OUTPUT_TOKENS: i64 = 16;
pub const DEFAULT_AUDIO_TOKEN_ESTIMATE: i64 = 300;
pub const AUDIO_BYTES_PER_TOKEN: i64 = 1600;
pub const RESPONSES_API_CALL_TYPES: [&str; 2] = ["aresponses", "responses"];
pub const EMBEDDING_API_CALL_TYPES: [&str; 2] = ["aembedding", "embedding"];
pub const TEXT_COMPLETION_API_CALL_TYPES: [&str; 2] = ["atext_completion", "text_completion"];
pub const RERANK_API_CALL_TYPES: [&str; 2] = ["rerank", "arerank"];
pub const GOOGLE_GENAI_NATIVE_CALL_TYPES: [&str; 4] = [
    "generate_content",
    "agenerate_content",
    "generate_content_stream",
    "agenerate_content_stream",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitType {
    Requests,
    Tokens,
    MaxParallelRequests,
}
impl RateLimitType {
    // the key builder and the serde wire form spell the same member, so the test keeps them equal
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requests => "requests",
            Self::Tokens => "tokens",
            Self::MaxParallelRequests => "max_parallel_requests",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowRateLimitType {
    Requests,
    Tokens,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReservationBackend {
    Redis,
    Local,
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RateLimitDescriptorRateLimit {
    pub requests_per_unit: Option<i64>,
    pub tokens_per_unit: Option<i64>,
    pub max_parallel_requests: Option<i64>,
    pub window_size: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RateLimitDescriptor {
    pub key: String,
    pub value: String,
    pub rate_limit: Option<RateLimitDescriptorRateLimit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParallelRequestGauge {
    pub counter_key: String,
    pub limit: i64,
    pub descriptor_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParallelSlotAcquisition {
    pub slot_id: String,
    pub counter_keys: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RateLimitStatus {
    pub code: String,
    pub current_limit: i64,
    pub limit_remaining: i64,
    pub rate_limit_type: RateLimitType,
    pub descriptor_key: String,
    pub descriptor_value: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ReservationWindowIdentity {
    pub counter_key: String,
    pub window_start: String,
    pub backend: ReservationBackend,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RateLimitResponse {
    pub overall_code: String,
    pub statuses: Vec<RateLimitStatus>,
    pub reservation_windows: Option<BTreeSet<ReservationWindowIdentity>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResponseWithDescriptors {
    pub descriptors: Vec<RateLimitDescriptor>,
    pub response: RateLimitResponse,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowKeyMetadata {
    pub requests_limit: Option<i64>,
    pub tokens_limit: Option<i64>,
    pub window_size: i64,
    pub descriptor_key: String,
    pub descriptor_value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AtomicCounterMeta {
    pub descriptor_key: String,
    pub descriptor_value: String,
    pub current_limit: i64,
    pub rate_limit_type: WindowRateLimitType,
    pub window_key: String,
    pub counter_key: String,
    pub increment: i64,
    pub ttl: i64,
    pub window_size: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AtomicCounterState {
    pub window_expired: bool,
    pub current: i64,
    pub window_start: String,
}

pub type DescriptorAtomicGroup = (Vec<String>, Vec<i64>, Vec<AtomicCounterMeta>);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CounterRefund {
    pub window_key: String,
    pub counter_key: String,
    pub window_start: String,
    pub increment: i64,
}

#[derive(Clone, Debug)]
pub struct ReservationAwareIncrement {
    pub operation: IncrementOperation,
    pub window_key: Option<String>,
    pub expected_window_start: Option<String>,
    pub reservation_backend: Option<ReservationBackend>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CacheCounterValue {
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptReply {
    Nil,
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Array(Vec<Self>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptReplyKind {
    Nil,
    Int,
    Float,
    Text,
    Bytes,
    Array,
}

impl ScriptReply {
    #[must_use]
    pub const fn kind(&self) -> ScriptReplyKind {
        match self {
            Self::Nil => ScriptReplyKind::Nil,
            Self::Int(_) => ScriptReplyKind::Int,
            Self::Float(_) => ScriptReplyKind::Float,
            Self::Text(_) => ScriptReplyKind::Text,
            Self::Bytes(_) => ScriptReplyKind::Bytes,
            Self::Array(_) => ScriptReplyKind::Array,
        }
    }
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum CounterReplyError {
    #[error("rate limiter script reply is not an array: {kind:?}")]
    NotAnArray { kind: ScriptReplyKind },
    #[error("rate limiter script reply holds a nil or a nested array: {kind:?}")]
    UnsupportedElement { kind: ScriptReplyKind },
}

pub fn counter_values_from_reply(
    reply: &ScriptReply,
) -> Result<Vec<CacheCounterValue>, CounterReplyError> {
    let ScriptReply::Array(elements) = reply else {
        return Err(CounterReplyError::NotAnArray { kind: reply.kind() });
    };
    let mut values = Vec::with_capacity(elements.len());
    for element in elements {
        let value = match element {
            ScriptReply::Int(number) => CacheCounterValue::Int(*number),
            ScriptReply::Float(number) => CacheCounterValue::Float(*number),
            ScriptReply::Text(text) => CacheCounterValue::Text(text.clone()),
            ScriptReply::Bytes(bytes) => CacheCounterValue::Bytes(bytes.clone()),
            other @ (ScriptReply::Nil | ScriptReply::Array(_)) => {
                return Err(CounterReplyError::UnsupportedElement { kind: other.kind() });
            }
        };
        values.push(value);
    }
    Ok(values)
}

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn create_rate_limit_keys(key: &str, value: &str, rate_limit_type: RateLimitType) -> String {
    format!("{{{key}:{value}}}:{}", rate_limit_type.as_str())
}

#[must_use]
pub fn sibling_counter_keys(window_key: &str) -> (String, String) {
    let prefix = window_key.strip_suffix(":window").unwrap_or(window_key);
    (format!("{prefix}:requests"), format!("{prefix}:tokens"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitedModel {
    pub requested: String,
    pub group: String,
}

impl RateLimitedModel {
    // a requested-name limit of 0 is a limit, so only an absent entry falls through to the group
    #[must_use]
    pub fn limit_in(&self, limits: Option<&BTreeMap<String, i64>>) -> Option<i64> {
        let configured = limits?;
        configured
            .get(&self.requested)
            .or_else(|| configured.get(&self.group))
            .copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TagRateLimit {
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
}

#[async_trait::async_trait]
pub trait TagRateLimitResolver: Send + Sync {
    async fn resolve(&self, tag_names: &[String]) -> BTreeMap<String, TagRateLimit>;
}

pub trait ModelGroupResolver: Send + Sync {
    fn resolve(&self, model: &str) -> Option<String>;
}

pub trait LimiterClock: Send + Sync {
    fn current_time(&self) -> SystemTime;
}
