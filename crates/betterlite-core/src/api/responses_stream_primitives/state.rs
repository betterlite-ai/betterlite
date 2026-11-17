use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::api_base_lookup::{
    ApiBaseContext, ApiBaseError, ApiBaseParams, get_api_base,
};
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

pub struct StreamTimes {
    pub now: DateTime<FixedOffset>,
    pub created_timestamp: f64,
}

pub struct LoggingSnapshot {
    pub start_time: Option<DateTime<FixedOffset>>,
    pub litellm_params: IndexMap<String, Json>,
}

impl LoggingSnapshot {
    pub fn read(logging: &StreamLoggingObject) -> Self {
        Self {
            start_time: logging.read(&mut |state| state.start_time),
            litellm_params: logging
                .read(&mut |state| state.litellm_params.clone())
                .unwrap_or_default(),
        }
    }
}

pub trait ResponsesApiBase {
    fn resolve(&self, model: &str, params: &ApiBaseParams) -> Result<Option<String>, ApiBaseError>;
}

pub struct SharedResponsesApiBase<'context> {
    pub context: ApiBaseContext<'context>,
}

impl ResponsesApiBase for SharedResponsesApiBase<'_> {
    fn resolve(&self, model: &str, params: &ApiBaseParams) -> Result<Option<String>, ApiBaseError> {
        get_api_base(model, params, &self.context)
    }
}

pub struct ResponsesStreamHiddenParams {
    pub model_id: Option<String>,
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub additional_headers: AdditionalHeaders,
}

pub struct BaseStreamStateInput<'input> {
    pub model: &'input str,
    pub custom_llm_provider: Option<&'input str>,
    pub litellm_metadata: Option<&'input IndexMap<String, Json>>,
    pub request_data: Option<&'input IndexMap<String, Json>>,
    pub call_type: Option<&'input str>,
    pub logging: LoggingSnapshot,
    pub headers: &'input IndexMap<String, String>,
    pub api_base: &'input dyn ResponsesApiBase,
    pub times: StreamTimes,
    pub max_streaming_duration_seconds: Option<f64>,
}

pub struct ResponsesStreamBaseState {
    pub model: String,
    pub custom_llm_provider: Option<String>,
    pub litellm_metadata: Option<IndexMap<String, Json>>,
    pub request_data: IndexMap<String, Json>,
    pub call_type: Option<String>,
    pub finished: bool,
    pub failure_handled: bool,
    pub yielded_first_chunk: bool,
    pub output_started: bool,
    pub completed_response: Option<ResponsesAPIStreamingResponse>,
    pub completed_response_cached: bool,
    pub completed_response_logged: bool,
    pub completed_response_cache_hit: Option<bool>,
    pub persist_completed_response_before_logging: bool,
    pub generated_content: String,
    pub generated_tool_arguments: String,
    pub start_time: DateTime<FixedOffset>,
    pub created_timestamp: f64,
    pub max_streaming_duration_seconds: Option<f64>,
    pub hidden_params: ResponsesStreamHiddenParams,
    pub raw_response_headers: IndexMap<String, String>,
    pub pending_tasks: TaskTracker,
}

pub fn base_stream_state(input: &BaseStreamStateInput<'_>) -> ResponsesStreamBaseState {
    let params = api_base_params(&input.logging.litellm_params);
    let api_base = input
        .api_base
        .resolve(input.model, &params)
        .unwrap_or_else(|error| {
            tracing::debug!(error = %error, "the responses stream api base could not be resolved");
            None
        });
    ResponsesStreamBaseState {
        model: input.model.to_owned(),
        custom_llm_provider: input.custom_llm_provider.map(str::to_owned),
        litellm_metadata: input.litellm_metadata.cloned(),
        request_data: input.request_data.cloned().unwrap_or_default(),
        call_type: input.call_type.map(str::to_owned),
        finished: false,
        failure_handled: false,
        yielded_first_chunk: false,
        output_started: false,
        completed_response: None,
        completed_response_cached: false,
        completed_response_logged: false,
        completed_response_cache_hit: None,
        persist_completed_response_before_logging: true,
        generated_content: String::new(),
        generated_tool_arguments: String::new(),
        start_time: input.logging.start_time.unwrap_or(input.times.now),
        created_timestamp: input.times.created_timestamp,
        max_streaming_duration_seconds: input.max_streaming_duration_seconds,
        hidden_params: ResponsesStreamHiddenParams {
            model_id: model_id_from_metadata(input.litellm_metadata),
            api_base,
            custom_llm_provider: input.custom_llm_provider.map(str::to_owned),
            additional_headers: process_response_headers(HeaderSource::Upstream(input.headers)),
        },
        raw_response_headers: input.headers.clone(),
        pending_tasks: TaskTracker::new(),
    }
}

// only a string id survives, so model_info={id: ""} keeps the empty string and a numeric id vanishes
fn model_id_from_metadata(metadata: Option<&IndexMap<String, Json>>) -> Option<String> {
    let Some(Json::Object(model_info)) = metadata?.get("model_info") else {
        return None;
    };
    match model_info.get("id") {
        Some(Json::Text(id)) => Some(id.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

// get_api_base reads these keys off model_call_details["litellm_params"], ignoring any other type
fn api_base_params(fields: &IndexMap<String, Json>) -> ApiBaseParams {
    ApiBaseParams {
        api_base: text_field(fields, "api_base"),
        base_url: text_field(fields, "base_url"),
        custom_llm_provider: text_field(fields, "custom_llm_provider"),
        api_key: text_field(fields, "api_key").map(SecretString::from),
        vertex_location: text_field(fields, "vertex_location"),
        vertex_project: text_field(fields, "vertex_project"),
        // getattr(optional_params, "stream", False) reads the dict itself, so the default base is never streaming
        stream: false,
    }
}

fn text_field(fields: &IndexMap<String, Json>, name: &str) -> Option<String> {
    match fields.get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}
