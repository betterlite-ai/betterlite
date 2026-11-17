use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::fallback_observations::error_info::FallbackErrorInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    #[serde(default)]
    pub details: Vec<DetailCase>,
    #[serde(default)]
    pub no_fallback: Vec<NoFallbackCase>,
    #[serde(default)]
    pub outcomes: Vec<OutcomeCase>,
    #[serde(default)]
    pub headers: Vec<HeaderCase>,
    #[serde(default)]
    pub events: Vec<EventCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub(super) enum DetailCase {
    RateLimit {
        message: String,
        #[serde(default)]
        body_param: Option<String>,
    },
    BadRequest {
        message: String,
        #[serde(default)]
        body_type: Option<String>,
        #[serde(default)]
        body_param: Option<String>,
    },
    Budget {
        #[serde(default)]
        message: Option<String>,
        current_cost: String,
        max_budget: String,
    },
    Guardrail {
        message: String,
        guardrail_name: Option<String>,
    },
    JsonSchema {
        raw_response: String,
        json_schema: String,
    },
    Plain {
        message: String,
        // the Python class name is the one field a ValueError makes differ, which betterlite's single unclassified family cannot hold
        #[serde(default)]
        exception_class: ExceptionClass,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ExceptionClass {
    #[default]
    Exception,
    ValueError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoFallbackCase {
    pub lookup_groups: Vec<String>,
    pub fallbacks: Vec<JsonObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OutcomeCase {
    pub model_group: Option<String>,
    pub fallback_model_group: Option<Vec<Json>>,
    pub fallback_failure_detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ResponseKind {
    Absent,
    Object,
    Attributed,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HeaderCase {
    pub kind: ResponseKind,
    pub attempted_fallbacks: u32,
    #[serde(default)]
    pub errors: Option<Vec<FallbackErrorInfo>>,
    #[serde(default)]
    pub existing: Option<JsonObject>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ObserverKind {
    Record,
    Fail,
    Silent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Outcome {
    Success,
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventCase {
    pub observers: Vec<ObserverKind>,
    pub outcome: Outcome,
    pub original_model_group: String,
    pub request: JsonObject,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Output {
    pub details: Vec<DetailFound>,
    pub no_fallback: Vec<String>,
    pub outcomes: Vec<String>,
    pub headers: Vec<HeaderFound>,
    pub events: Vec<EventFound>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DetailFound {
    pub info: FallbackErrorInfo,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HeaderFound {
    pub result: ResponseKind,
    pub has_hidden_params: bool,
    pub headers: JsonObject,
    pub errors_present: bool,
    pub errors: Vec<JsonObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventFound {
    pub calls: Vec<EventCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventCall {
    pub observer: u32,
    pub outcome: Outcome,
    pub original_model_group: String,
    pub model: Option<String>,
    pub message: String,
}
