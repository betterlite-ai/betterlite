use betterlite_logging::logging_service_events::clock::SystemClock;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::{HiddenParams, Json as DecisionJson};
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use std::sync::Arc;

use crate::api::vector_store_http_write::error::header_text;
use crate::genai_setup::params::GenerateContentLiteLLMParams;
use crate::google_genai::logging::GenAiStreamLogging;
use crate::google_genai::metadata::GenAiStreamMetadata;
use crate::google_genai::provider::GenAiProvider;

const DEFAULT_LINE_LIMIT: usize = 0x0010_0000;

// The pieces an async iterator's EOF logging takes: the injected clock, the logging wiring and the per-line bound.
pub struct StreamSettings {
    pub line_limit: usize,
    pub clock: Arc<dyn SuccessClock>,
    pub logging: Option<GenAiStreamLogging>,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            line_limit: DEFAULT_LINE_LIMIT,
            clock: Arc::new(SystemClock),
            logging: None,
        }
    }
}

// What GoogleGenAIGenerateContentStreamingIterator stores beside the hidden params
pub struct StreamInputs<'call> {
    pub model: &'call str,
    pub provider: &'call str,
    pub body: Object,
    pub params: &'call GenerateContentLiteLLMParams,
    pub api_base: String,
}

pub(super) fn stream_metadata(
    inputs: &StreamInputs<'_>,
    headers: &http::HeaderMap,
) -> GenAiStreamMetadata {
    GenAiStreamMetadata {
        model: inputs.model.to_owned(),
        provider: GenAiProvider::from_spelling(inputs.provider),
        request_body: decision_object(&inputs.body),
        hidden_params: hidden_params(inputs, headers),
    }
}

// _google_genai_streaming_hidden_params: model_info's id, else the request model, with B206 over the reply headers
fn hidden_params(inputs: &StreamInputs<'_>, headers: &http::HeaderMap) -> HiddenParams {
    let mut params = HiddenParams::new();
    params.insert("model_id".to_owned(), DecisionJson::Text(model_id(inputs)));
    params.insert(
        "api_base".to_owned(),
        DecisionJson::Text(inputs.api_base.clone()),
    );
    params.insert("cache_key".to_owned(), DecisionJson::Text(String::new()));
    params.insert(
        "response_cost".to_owned(),
        DecisionJson::Text(String::new()),
    );
    params.insert(
        "additional_headers".to_owned(),
        DecisionJson::Object(process_response_headers(HeaderSource::Upstream(
            &header_index(headers),
        ))),
    );
    params
}

fn model_id(inputs: &StreamInputs<'_>) -> String {
    let id = inputs
        .params
        .extra
        .get("model_info")
        .and_then(Json::as_object)
        .and_then(|info| info.get("id"));
    match id {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => inputs.model.to_owned(),
    }
}

pub(super) fn header_index(headers: &http::HeaderMap) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}

pub(super) fn decision_object(object: &Object) -> IndexMap<String, DecisionJson> {
    object
        .iter()
        .map(|(name, value)| (name.clone(), decision_json(value)))
        .collect()
}

pub(super) fn decision_json(value: &Json) -> DecisionJson {
    match value {
        Json::Null => DecisionJson::Null,
        Json::Bool(flag) => DecisionJson::Bool(*flag),
        Json::Number(number) => DecisionJson::Number(number.clone()),
        Json::Text(text) => DecisionJson::Text(text.clone()),
        Json::List(items) => DecisionJson::List(items.iter().map(decision_json).collect()),
        Json::Object(fields) => DecisionJson::Object(decision_object(fields)),
    }
}
