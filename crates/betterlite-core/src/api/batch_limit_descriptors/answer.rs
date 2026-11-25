use betterlite_models::model_registration::JsonObject;
use serde::Serialize;

use super::descriptors::{BatchDescriptorIncrements, BatchLimitDescriptor};

// One answer per owned callable, tagged with the callable's name so a case reads both sides the same way
#[derive(Debug, Serialize)]
pub struct BatchLimitDescriptorsResult {
    pub answers: Vec<BatchLimitAnswer>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub enum BatchLimitAnswer {
    FileBoundModel {
        model: Option<String>,
    },
    BatchRoutingModel {
        model: Option<String>,
    },
    ResolveBatchProvider {
        provider: Option<String>,
    },
    #[serde(rename = "create_batch_rate_limit_descriptors")]
    CreateRateLimitDescriptors {
        descriptors: Vec<BatchLimitDescriptor>,
    },
    ProjectHasIoLimits {
        has_limits: bool,
    },
    CreateProjectIoDescriptors {
        descriptors: Vec<BatchLimitDescriptor>,
        increments: Vec<BatchDescriptorIncrements>,
    },
    ShouldSkip {
        skip: bool,
        descriptors: Option<Vec<BatchLimitDescriptor>>,
    },
    KeyRequiresAccessCheck {
        required: bool,
    },
    EstimateOutputTokens {
        tokens: i64,
    },
    HasApplicableLimits {
        applicable: bool,
    },
    #[serde(rename = "resolve_batch_input_file_fetch_params")]
    ResolveFetchParams {
        file_id: String,
        params: JsonObject,
    },
}
