use std::collections::BTreeMap;

use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenScope;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_openai::batches::records::BatchStatus;
use betterlite_rate_limit::limiter_records::ReservationBackend;
use betterlite_rate_limit::limiter_records::{ParallelSlotAcquisition, RateLimitStatus};
use betterlite_rate_limit::reconciliation_operations::scope_targets::ScopeMetadata;
use betterlite_rate_limit::usage_resolution::Usage;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct WindowCase {
    pub(crate) counter_key: String,
    pub(crate) window_start: String,
    pub(crate) backend: ReservationBackend,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct TpdOperationCase {
    pub(crate) key: String,
    pub(crate) increment_value: f64,
    pub(crate) ttl: Option<i64>,
    pub(crate) window_key: Option<String>,
    pub(crate) expected_window_start: Option<String>,
    pub(crate) reservation_backend: Option<ReservationBackend>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct StashCase {
    pub(crate) owner_call_id: Option<String>,
    pub(crate) parallel_slot: Option<ParallelSlotAcquisition>,
    pub(crate) statuses: Vec<RateLimitStatus>,
    pub(crate) reserved_tokens: i64,
    pub(crate) reserved_scopes: Vec<(String, String)>,
    pub(crate) itpm_reserved_tokens: i64,
    pub(crate) itpm_reserved_scopes: Vec<(String, String)>,
    pub(crate) itpm_reserved_windows: Vec<WindowCase>,
    pub(crate) otpm_reserved_tokens: i64,
    pub(crate) otpm_reserved_scopes: Vec<(String, String)>,
    pub(crate) otpm_reserved_windows: Vec<WindowCase>,
    pub(crate) batch_tokens: Option<i64>,
    pub(crate) batch_scopes: Vec<BatchEnqueuedTokenScope>,
    pub(crate) batch_tpd_ops: Vec<TpdOperationCase>,
    pub(crate) reservation_released: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct HiddenParamsCase {
    pub(crate) additional_headers: AdditionalHeaders,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ResponseCase {
    pub(crate) id: String,
    pub(crate) status: Option<BatchStatus>,
    pub(crate) object: Option<String>,
    #[serde(rename = "_hidden_params")]
    pub(crate) hidden_params: Option<HiddenParamsCase>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Case {
    pub(crate) call_id: Option<String>,
    pub(crate) stash: Option<StashCase>,
    pub(crate) headers: AdditionalHeaders,
    pub(crate) usage: Option<Usage>,
    pub(crate) logging: ScopeMetadata,
    pub(crate) response: Option<ResponseCase>,
    pub(crate) pop_tokens: Option<i64>,
    pub(crate) runs: Option<usize>,
    pub(crate) seed: BTreeMap<String, i64>,
}
