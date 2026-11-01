use serde::{Deserialize, Serialize};

use crate::baseline_history::cache_plan::CountedPromptCachePlan;
use crate::baseline_history::error::ObservationError;
use crate::baseline_history::usage::Usage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Complete,
    Uncertain,
    ResponseCache,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default = "default_version")]
    version: i64,
    request_id: String,
    started_at: f64,
    available_at: f64,
    outcome: Outcome,
    baseline_equivalent: bool,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    plan: Option<CountedPromptCachePlan>,
    #[serde(default)]
    minimum_cache_tokens: i64,
    #[serde(default)]
    reason: Option<String>,
}

const fn default_version() -> i64 {
    3
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(try_from = "Raw")]
pub struct BaselineObservation {
    pub request_id: String,
    pub started_at: f64,
    pub available_at: f64,
    pub outcome: Outcome,
    pub baseline_equivalent: bool,
    pub usage: Option<Usage>,
    pub plan: Option<CountedPromptCachePlan>,
    pub minimum_cache_tokens: i64,
    pub reason: Option<String>,
}

impl TryFrom<Raw> for BaselineObservation {
    type Error = ObservationError;

    fn try_from(raw: Raw) -> Result<Self, Self::Error> {
        if raw.version != 3 {
            return Err(ObservationError::Version);
        }
        if raw.request_id.is_empty() {
            return Err(ObservationError::EmptyRequestId);
        }
        if !raw.started_at.is_finite() || raw.started_at < 0.0_f64 {
            return Err(ObservationError::InvalidStartedAt);
        }
        if !raw.available_at.is_finite() || raw.available_at < 0.0_f64 {
            return Err(ObservationError::InvalidAvailableAt);
        }
        if raw.minimum_cache_tokens < 0 {
            return Err(ObservationError::NegativeMinimumCacheTokens);
        }
        Ok(Self {
            request_id: raw.request_id,
            started_at: raw.started_at,
            available_at: raw.available_at,
            outcome: raw.outcome,
            baseline_equivalent: raw.baseline_equivalent,
            usage: raw.usage,
            plan: raw.plan,
            minimum_cache_tokens: raw.minimum_cache_tokens,
            reason: raw.reason,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheEntry {
    pub fingerprint: String,
    pub content_fingerprint: String,
    pub tokens: i64,
    pub ttl_seconds: i64,
    pub available_at: f64,
    pub expires_at: f64,
    #[serde(default)]
    pub uncertain: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct BaselineHistory {
    pub first_at: Option<f64>,
    pub last_at: Option<f64>,
    pub equivalent: bool,
    pub uncertain_before: f64,
    pub entries: Vec<CacheEntry>,
    pub blocked_until: f64,
}

impl Default for BaselineHistory {
    fn default() -> Self {
        Self {
            first_at: None,
            last_at: None,
            equivalent: true,
            uncertain_before: 0.0,
            entries: Vec::new(),
            blocked_until: 0.0,
        }
    }
}

mod tests;
