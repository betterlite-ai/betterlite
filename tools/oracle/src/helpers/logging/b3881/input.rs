use std::collections::BTreeMap;

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Function {
    Failure,
    FallbackSuccess,
    FallbackFailure,
    Success,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) function: Function,
    #[serde(default)]
    pub(super) kwargs: JsonObject,
    #[serde(default)]
    pub(super) payload: Option<StandardLoggingPayload>,
    #[serde(default)]
    pub(super) exception: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) values: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) original_model_group: String,
    #[serde(default)]
    pub(super) start_time: Option<String>,
    #[serde(default)]
    pub(super) completion_start_time: Option<String>,
    #[serde(default)]
    pub(super) end_time: Option<String>,
    #[serde(default)]
    pub(super) output_tokens: Option<String>,
}
