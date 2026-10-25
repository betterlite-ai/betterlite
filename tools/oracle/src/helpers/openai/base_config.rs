use crate::helpers::{HelperError, HelperResult};
use betterlite_openai::openai_base_config::forwarding::{
    BaseConfigError, ForwardingFailure, OpenAiErrorParts, OpenAiErrorRecord, ResponseForward,
    forward_response,
};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, OpenAiConfig, OpenAiSettings, is_gpt_reasoning_series_name,
    is_o_series_model, metadata_filter_limit, overlapping_api_base_path,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
type Object = Map<String, Value>;
mod tests;
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigInput { #[serde(default)] family: ConfigFamily, api_key: Option<String>, #[serde(default)] headers: BTreeMap<String, String> }
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoutingInput {
    model: String,
    #[serde(default)]
    chat_models: BTreeSet<String>,
    #[serde(default)]
    text_models: BTreeSet<String>,
}
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportedInput { model: String, #[serde(default)] chat_models: BTreeSet<String>, #[serde(default)] text_models: BTreeSet<String>, #[serde(flatten)] config: ConfigInput }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingInput { model: String, #[serde(default)] chat_models: BTreeSet<String>, #[serde(default)] text_models: BTreeSet<String>, #[serde(flatten)] config: ConfigInput, #[serde(default)] existing: Object, #[serde(default)] requested: Object }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthInput { model: String, #[serde(flatten)] config: ConfigInput }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput { model: String, messages: Value, optional: Object, #[serde(flatten)] config: ConfigInput }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectionInput { raw_text: String, #[serde(default)] raw_headers: BTreeMap<String, String> }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorInput { status_code: u16, message: String, headers: Option<BTreeMap<String, String>>, body: Option<Object> }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataInput { metadata: Option<IndexMap<String, Value>> }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverlapInput { api_base: String, ending_path: String }
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BaseCall {
    OSeriesRouting(RoutingInput),
    Gpt5Routing(RoutingInput),
    AudioRouting(RoutingInput),
    CatalogUserOnly(SupportedInput),
    OldGptResponseFormatDrop(SupportedInput),
    UnknownReasoningEffort(SupportedInput),
    BaseParameterAllowlist(MappingInput),
    BaseAuthOverride(AuthInput),
    BaseProjectionForwarding(ProjectionInput),
    BaseRequestOverlay(RequestInput),
    OpenaiErrorDefaults(ErrorInput),
    MetadataFilterLimit(MetadataInput),
    OverlappingApiBasePath(OverlapInput),
    AzureGpt5RoutePredicate(RoutingInput),
}
fn config(input: &ConfigInput) -> OpenAiConfig {
    let api_key = input.api_key.clone().map(secrecy::SecretString::from);
    let headers = input.headers.clone();
    OpenAiConfig {
        family: input.family,
        api_key,
        headers,
        settings: OpenAiSettings::default(),
    }
}
fn invalid_error(error: impl std::fmt::Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
fn error_value(input: &ErrorInput) -> Result<Value, Box<HelperError>> {
    let record = OpenAiErrorRecord::from_parts(OpenAiErrorParts {
        status_code: input.status_code,
        message: input.message.clone(),
        headers: input.headers.clone(),
        request: None,
        response: None,
        body: input.body.clone(),
    });
    serde_json::to_value(record).map_err(invalid_error)
}
fn forwarding(input: &ProjectionInput) -> Value {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut wire_headers = input.raw_headers.clone();
    wire_headers.insert(
        "content-length".to_owned(),
        input.raw_text.len().to_string(),
    );
    wire_headers.insert(
        "content-type".to_owned(),
        "text/plain; charset=utf-8".to_owned(),
    );
    let sink_events = Rc::clone(&events);
    let projection_events = Rc::clone(&events);
    let response = forward_response(
        |raw, callback_headers| {
            sink_events
                .borrow_mut()
                .push(json!(["post_call", raw, callback_headers]));
            Ok::<(), ForwardingFailure>(())
        },
        |decoded, callback_headers, hidden_headers| {
            projection_events
                .borrow_mut()
                .push(json!([decoded, hidden_headers, callback_headers]));
            Ok::<Value, ForwardingFailure>(json!("projected"))
        },
        &ResponseForward {
            raw_text: &input.raw_text,
            raw_headers: &wire_headers,
            hidden_headers: &wire_headers,
        },
    );
    match response {
        Ok(_value) => json!({"events": events.take()}),
        Err(BaseConfigError::Decode(_)) => {
            json!({"events": events.take(), "error": "JSONDecodeError"})
        }
        Err(error) => json!({"events": events.take(), "error": error.to_string()}),
    }
}
fn mapped(value: &MappingInput) -> Value {
    let mut existing = value.existing.clone();
    config(&value.config).map_params(
        &value.model,
        &value.requested,
        &mut existing,
        &CatalogCapabilities {
            chat_models: value.chat_models.clone(),
            text_models: value.text_models.clone(),
        },
    );
    Value::Object(existing)
}
fn invoke(input: &BaseCall) -> Result<Value, Box<HelperError>> {
    match input {
        BaseCall::OSeriesRouting(value) => Ok(json!(is_o_series_model(
            &value.model,
            &CatalogCapabilities {
                chat_models: value.chat_models.clone(),
                text_models: value.text_models.clone()
            }
        ))),
        BaseCall::Gpt5Routing(value) => Ok(json!(is_gpt_reasoning_series_name(&value.model))),
        BaseCall::AudioRouting(value) => Ok(json!(
            value.chat_models.contains(&value.model) && value.model.contains("audio")
        )),
        BaseCall::AzureGpt5RoutePredicate(value) => Ok(json!(
            is_gpt_reasoning_series_name(&value.model) || value.model.contains("gpt5_series")
        )),
        BaseCall::CatalogUserOnly(value)
        | BaseCall::OldGptResponseFormatDrop(value)
        | BaseCall::UnknownReasoningEffort(value) => {
            Ok(json!(config(&value.config).supported_params(
                &value.model,
                &CatalogCapabilities {
                    chat_models: value.chat_models.clone(),
                    text_models: value.text_models.clone()
                }
            )))
        }
        BaseCall::BaseParameterAllowlist(value) => Ok(mapped(value)),
        BaseCall::BaseAuthOverride(value) => {
            Ok(json!(config(&value.config).validate_environment()))
        }
        BaseCall::BaseProjectionForwarding(value) => Ok(forwarding(value)),
        BaseCall::BaseRequestOverlay(value) => Ok(json!(config(&value.config).request_body(
            &value.model,
            value.messages.clone(),
            &value.optional
        ))),
        BaseCall::OpenaiErrorDefaults(value) => error_value(value),
        BaseCall::MetadataFilterLimit(value) => {
            serde_json::to_value(metadata_filter_limit(value.metadata.as_ref()))
                .map_err(invalid_error)
        }
        BaseCall::OverlappingApiBasePath(value) => {
            overlapping_api_base_path(&value.api_base, &value.ending_path)
                .map(Value::String)
                .map_err(invalid_error)
        }
    }
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = BaseCall;
    fn run(input: &Self::Input) -> HelperResult {
        invoke(input)
    }
}
