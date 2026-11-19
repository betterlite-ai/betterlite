use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;

// Anthropic's two cache retention tiers, in the seconds LiteLLM compares a prefix ttl against.
pub const FIVE_MINUTE_TTL_SECONDS: u32 = 300;
pub const ONE_HOUR_TTL_SECONDS: u32 = 3600;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    Warm,
    Partial,
    Stale,
    #[default]
    Unknown,
    Disabled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountSource {
    AnthropicCountTokens,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheTokenBuckets {
    pub uncached_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_5m_input_tokens: i64,
    pub cache_creation_1h_input_tokens: i64,
}

impl CacheTokenBuckets {
    #[must_use]
    pub const fn total_tokens(&self) -> i64 {
        self.uncached_input_tokens
            .saturating_add(self.cache_read_input_tokens)
            .saturating_add(self.cache_creation_5m_input_tokens)
            .saturating_add(self.cache_creation_1h_input_tokens)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheEvidenceSource {
    #[default]
    ProviderUsage,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheEvidenceConfidence {
    #[default]
    Observed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CacheEvidence {
    pub observed_at: f64,
    pub expires_at: f64,
    #[serde(default)]
    pub source: CacheEvidenceSource,
    #[serde(default)]
    pub confidence: CacheEvidenceConfidence,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct CacheCostScenario {
    pub tokens: CacheTokenBuckets,
    pub input_cost: Decimal,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CachePredictionArm {
    pub deployment_id: String,
    pub model: Option<String>,
    #[serde(default)]
    pub cache_state: CacheState,
    pub reason: Option<String>,
    pub estimate: Option<CacheCostScenario>,
    pub cold: Option<CacheCostScenario>,
    pub warm: Option<CacheCostScenario>,
    pub evidence: Option<CacheEvidence>,
    pub token_count_source: Option<TokenCountSource>,
}
