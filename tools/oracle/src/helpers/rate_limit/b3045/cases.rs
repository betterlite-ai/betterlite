use std::collections::BTreeMap;

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;
use betterlite_rate_limit::limiter_records::ReservationBackend;
use betterlite_rate_limit::reconciliation_operations::scope_targets::ScopeMetadata;
use betterlite_rate_limit::usage_resolution::{ResponseAPIUsage, Usage};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseKind {
    #[default]
    Missing,
    Usage,
    ResponseApi,
    Mapping,
    WithUsage,
    Rerank,
    Unmodelled,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenPair {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ResponseCase {
    #[serde(default)]
    pub(crate) kind: ResponseKind,
    #[serde(default)]
    pub(crate) usage: Option<Usage>,
    #[serde(default)]
    pub(crate) response_api_usage: Option<ResponseAPIUsage>,
    #[serde(default)]
    pub(crate) rerank_tokens: Option<TokenPair>,
    #[serde(default)]
    pub(crate) billed_units_total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModelCase {
    pub(crate) requested: String,
    pub(crate) group: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WindowCase {
    pub(crate) counter_key: String,
    pub(crate) window_start: String,
    pub(crate) backend: ReservationBackend,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct StashCase {
    #[serde(default)]
    pub(crate) owner_call_id: Option<String>,
    #[serde(default)]
    pub(crate) reserved_tokens: i64,
    #[serde(default)]
    pub(crate) reserved_model: Option<ModelCase>,
    #[serde(default)]
    pub(crate) reserved_scopes: Vec<(String, String)>,
    #[serde(default)]
    pub(crate) itpm_reserved_tokens: i64,
    #[serde(default)]
    pub(crate) itpm_reserved_scopes: Vec<(String, String)>,
    #[serde(default)]
    pub(crate) itpm_reserved_windows: Vec<WindowCase>,
    #[serde(default)]
    pub(crate) otpm_reserved_tokens: i64,
    #[serde(default)]
    pub(crate) otpm_reserved_scopes: Vec<(String, String)>,
    #[serde(default)]
    pub(crate) otpm_reserved_windows: Vec<WindowCase>,
    #[serde(default)]
    pub(crate) reservation_released: bool,
    #[serde(default)]
    pub(crate) tpm_limited_tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IoTokenCase {
    #[serde(default)]
    pub(crate) call_id: Option<String>,
    #[serde(default)]
    pub(crate) stash: Option<StashCase>,
    #[serde(default)]
    pub(crate) response: ResponseCase,
    #[serde(default)]
    pub(crate) combined_usage: Option<Usage>,
    pub(crate) window_size: i64,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ScopeTargetsCase {
    #[serde(default)]
    pub(crate) logging: ScopeMetadata,
    #[serde(default)]
    pub(crate) request_user: Option<String>,
    #[serde(default)]
    pub(crate) model_group: Option<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SuccessCase {
    #[serde(default)]
    pub(crate) call_id: Option<String>,
    #[serde(default)]
    pub(crate) stash: Option<StashCase>,
    #[serde(default)]
    pub(crate) response: ResponseCase,
    #[serde(default)]
    pub(crate) combined_usage: Option<Usage>,
    #[serde(default)]
    pub(crate) rate_limit_type: TokenRateLimitType,
    #[serde(default)]
    pub(crate) logging: ScopeMetadata,
    #[serde(default)]
    pub(crate) request_metadata: MetadataMap,
    #[serde(default)]
    pub(crate) request_user: Option<String>,
    #[serde(default)]
    pub(crate) model_group: Option<String>,
    #[serde(default)]
    pub(crate) groups: BTreeMap<String, String>,
    pub(crate) window_size: i64,
}
