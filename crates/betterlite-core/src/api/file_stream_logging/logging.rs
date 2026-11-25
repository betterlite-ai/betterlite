use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::fields::payload::records::StandardLoggingHiddenParams;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, FixedOffset};
use thiserror::Error;

// A callback raising propagates out of the stream like LiteLLM's logging object, after the completion flag is set
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct CallbackFailure {
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct SuccessCall {
    pub result: Json,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub standard_logging_object: Option<StandardLoggingPayload>,
}

#[derive(Debug, Clone)]
pub struct FailureCall {
    pub exception: ErrorClass,
    pub traceback: String,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

// LiteLLM's logging object methods the wrapper calls; the production implementation bridges B250 and B251
#[async_trait]
pub trait FileStreamLogger: Send + Sync {
    fn start_time(&self) -> Option<DateTime<FixedOffset>>;

    fn details_api_base(&self) -> Option<String>;

    fn build_payload(
        &self,
        response: &Json,
        start_time: DateTime<FixedOffset>,
        end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload>;

    fn success_handler(&self, call: SuccessCall) -> Result<(), CallbackFailure>;

    async fn async_success_handler(&self, call: SuccessCall) -> Result<(), CallbackFailure>;

    fn success_sync_callbacks(&self, call: SuccessCall) -> Result<(), CallbackFailure>;

    fn failure_handler(&self, call: FailureCall) -> Result<(), CallbackFailure>;

    async fn async_failure_handler(&self, call: FailureCall) -> Result<(), CallbackFailure>;
}

// A key of LiteLLM's _hidden_params: Absent never overrides the payload, Null overrides it with null
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HiddenField<Value> {
    #[default]
    Absent,
    Null,
    Value(Value),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenParams {
    pub api_base: HiddenField<String>,
    pub litellm_model_name: Option<String>,
    pub response_cost: HiddenField<Json>,
}

#[must_use]
pub fn logging_response(file_id: &str, model: Option<&str>) -> JsonObject {
    let mut response = JsonObject::new();
    response.insert(String::from("id"), Json::Text(file_id.to_owned()));
    response.insert(
        String::from("object"),
        Json::Text(String::from("file.content")),
    );
    if let Some(name) = model.filter(|value| !value.is_empty()) {
        response.insert(String::from("model"), Json::Text(name.to_owned()));
    }
    response
}

#[must_use]
pub fn merged_hidden(
    base: StandardLoggingHiddenParams,
    hidden: &HiddenParams,
) -> StandardLoggingHiddenParams {
    let mut merged = base;
    match &hidden.api_base {
        HiddenField::Absent => {}
        HiddenField::Null => merged.api_base = None,
        HiddenField::Value(value) => merged.api_base = Some(value.clone()),
    }
    merged
        .litellm_model_name
        .clone_from(&hidden.litellm_model_name);
    match &hidden.response_cost {
        HiddenField::Absent => {}
        HiddenField::Null => merged.response_cost = None,
        HiddenField::Value(value) => merged.response_cost = Some(value.clone()),
    }
    merged
}

#[must_use]
pub const fn hidden_api_base(hidden: &HiddenParams) -> Option<&str> {
    match &hidden.api_base {
        HiddenField::Value(value) => Some(value.as_str()),
        HiddenField::Absent | HiddenField::Null => None,
    }
}
