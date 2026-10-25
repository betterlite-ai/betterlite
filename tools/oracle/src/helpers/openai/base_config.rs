use crate::helpers::{HelperError, HelperResult};
use betterlite_openai::openai_base_config::forwarding::{
    BaseConfigError, ForwardingFailure, OpenAiErrorParts, OpenAiErrorRecord, ResponseForward,
    forward_response,
};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, OpenAiConfig, OpenAiSettings, is_azure_gpt5_model,
    is_gpt_audio_model, is_gpt_reasoning_series_name, is_o_series_model, metadata_filter_limit,
    overlapping_api_base_path,
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
pub(crate) struct CatalogInput { #[serde(default)] chat_models: BTreeSet<String>, #[serde(default)] text_models: BTreeSet<String> }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigInput { #[serde(default)] family: ConfigFamily, api_key: Option<String>, #[serde(default)] headers: BTreeMap<String, String> }
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoutingInput {
    model: String,
    #[serde(flatten)]
    catalog: CatalogInput,
}
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportedInput { model: String, #[serde(flatten)] catalog: CatalogInput, #[serde(flatten)] config: ConfigInput }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingInput { model: String, #[serde(flatten)] catalog: CatalogInput, #[serde(flatten)] config: ConfigInput, #[serde(default)] existing: Object, #[serde(default)] requested: Object }
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
pub(crate) struct ProjectionInput { raw_text: String, #[serde(default)] raw_headers: BTreeMap<String, String>, #[serde(default)] hidden_headers: BTreeMap<String, String>, decoded: Option<Value>, stream_value: Option<Value>, #[serde(default)] stream_error: bool }
#[rustfmt::skip]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorInput { #[serde(default = "default_status")] status_code: u16, message: String, headers: Option<BTreeMap<String, String>>, body: Option<Object> }
const fn default_status() -> u16 {
    match 0_u16 {
        0 => 500,
        value => value,
    }
}
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
fn catalog(input: &CatalogInput) -> CatalogCapabilities {
    let chat_models = input.chat_models.clone();
    let text_models = input.text_models.clone();
    CatalogCapabilities {
        chat_models,
        text_models,
    }
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
fn invalid_input(message: String) -> HelperError {
    let class = String::from("InvalidInput");
    HelperError {
        class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
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
    serde_json::to_value(record).map_err(|error| Box::new(invalid_input(error.to_string())))
}
fn forwarding(input: &ProjectionInput) -> Value {
    let events = Rc::new(RefCell::new(Vec::new()));
    let headers = input.raw_headers.clone();
    let hidden = input.hidden_headers.clone();
    let sink_events = Rc::clone(&events);
    let projection_events = Rc::clone(&events);
    let response = forward_response(
        |raw, response_headers| {
            sink_events
                .borrow_mut()
                .push(json!(["post_call", raw, response_headers]));
            Ok::<(), ForwardingFailure>(())
        },
        |decoded, response_headers, hidden_headers| {
            projection_events.borrow_mut().push(json!([
                "projection",
                decoded,
                response_headers,
                hidden_headers
            ]));
            Ok::<Value, ForwardingFailure>(json!("projected"))
        },
        ResponseForward {
            raw_text: &input.raw_text,
            raw_headers: &headers,
            decoded: serde_json::from_str(&input.raw_text),
            hidden_headers: &hidden,
        },
    );
    json!({"events": events.take(), "projected": projection_result(response), "stream": stream_result(input)})
}
fn projection_result(response: Result<Value, BaseConfigError>) -> Value {
    let decode_error = json!({"error": "decode"});
    response.map_or_else(|_| decode_error, |value| json!(value))
}
fn stream_result(input: &ProjectionInput) -> Value {
    let stream_value = input.stream_value.clone();
    stream_value.map_or_else(
        || Value::Null,
        |value| {
            if input.stream_error {
                json!({"error": "injected forwarding failure: stream"})
            } else {
                json!(value)
            }
        },
    )
}
fn supported(value: &SupportedInput) -> Value {
    let capabilities = catalog(&value.catalog);
    json!(config(&value.config).supported_params(&value.model, &capabilities))
}
fn mapped(value: &MappingInput) -> Value {
    let mut existing = value.existing.clone();
    config(&value.config).map_params(
        &value.model,
        &value.requested,
        &mut existing,
        &catalog(&value.catalog),
    );
    Value::Object(existing)
}
fn invoke(input: &BaseCall) -> Result<Value, Box<HelperError>> {
    match input {
        BaseCall::OSeriesRouting(value) => Ok(json!(is_o_series_model(
            &value.model,
            &catalog(&value.catalog)
        ))),
        BaseCall::Gpt5Routing(value) => Ok(json!(is_gpt_reasoning_series_name(&value.model))),
        BaseCall::AudioRouting(value) => Ok(json!(is_gpt_audio_model(
            &value.model,
            &catalog(&value.catalog)
        ))),
        BaseCall::AzureGpt5RoutePredicate(value) => Ok(json!(is_azure_gpt5_model(&value.model))),
        BaseCall::CatalogUserOnly(value)
        | BaseCall::OldGptResponseFormatDrop(value)
        | BaseCall::UnknownReasoningEffort(value) => Ok(supported(value)),
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
                .map_err(|error| Box::new(invalid_input(error.to_string())))
        }
        BaseCall::OverlappingApiBasePath(value) => {
            overlapping_api_base_path(&value.api_base, &value.ending_path)
                .map(Value::String)
                .map_err(|error| Box::new(invalid_input(error.to_string())))
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
