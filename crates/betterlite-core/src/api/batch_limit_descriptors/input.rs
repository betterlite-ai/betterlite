use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::descriptors::BatchLimitDescriptor;
use crate::api::request_destination::RequestData;

// The B052 principal wire record; the auth type keeps its constructors, so the record lands there first
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct BatchLimitPrincipal(pub BTreeMap<String, JsonData>);

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchLimitDescriptorsInput {
    pub principal: BatchLimitPrincipal,
    pub calls: Vec<BatchLimitCall>,
}

// One owned callable of the slice, with the request data or facts it reads
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub enum BatchLimitCall {
    FileBoundModel {
        data: RequestData,
    },
    BatchRoutingModel {
        data: RequestData,
    },
    ResolveBatchProvider {
        model: Option<String>,
    },
    #[serde(rename = "create_batch_rate_limit_descriptors")]
    CreateRateLimitDescriptors {
        data: RequestData,
    },
    ProjectHasIoLimits,
    CreateProjectIoDescriptors {
        per_model_usage: IndexMap<String, BatchModelUsage>,
    },
    ShouldSkip {
        data: RequestData,
        #[serde(default)]
        has_enqueued_scopes: bool,
    },
    KeyRequiresAccessCheck,
    EstimateOutputTokens {
        entry: RequestData,
        min_configured_otpm_limit: Option<i64>,
    },
    HasApplicableLimits {
        descriptors: Vec<BatchLimitDescriptor>,
    },
    #[serde(rename = "resolve_batch_input_file_fetch_params")]
    ResolveFetchParams {
        file_id: String,
        custom_llm_provider: String,
        data: RequestData,
    },
}

// A row's counted usage, keyed by the row's own body.model
#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatchModelUsage {
    pub total_tokens: i64,
    pub output_tokens: i64,
}
