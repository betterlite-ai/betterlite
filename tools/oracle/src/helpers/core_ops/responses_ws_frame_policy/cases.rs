use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use indexmap::IndexMap;
use serde::Deserialize;

use super::Replacements;
use super::fakes::Verdict;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuotaStep {
    pub(super) message: String,
    pub(super) authorized_model: Option<String>,
    pub(super) verdicts: Vec<Verdict>,
    pub(super) refuse_send: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateStep {
    pub(super) message: String,
    pub(super) authorized_model: Option<String>,
    pub(super) defaults: Option<WsRequestDefaults<Json>>,
    pub(super) guardrails: Replacements,
    pub(super) request_data: IndexMap<String, Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnmaskStep {
    pub(super) message: String,
    pub(super) guardrails: usize,
    pub(super) request_data: IndexMap<String, Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletedStep {
    pub(super) message: String,
    pub(super) guardrails: Replacements,
    pub(super) request_data: IndexMap<String, Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailureStep {
    pub(super) events: Vec<IndexMap<String, Json>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WrapStep {
    pub(super) litellm_metadata: Json,
    pub(super) event: IndexMap<String, Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetsStep {
    pub(super) candidates: Vec<String>,
}

// each step is boxed so every variant carries a pointer, well within the 3x variant_size_differences enforces
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Quota(Box<QuotaStep>),
    Create(Box<CreateStep>),
    Unmask(Box<UnmaskStep>),
    Completed(Box<CompletedStep>),
    Failure(Box<FailureStep>),
    Wrap(Box<WrapStep>),
    Sets(Box<SetsStep>),
}
