use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_character_calls::error::CharacterHandlerFailure;
use betterlite_core::api::video_character_calls::handler::{
    CharacterAnswered, CharacterHandler, CreateHandlerCall, GetHandlerCall,
};
use betterlite_core::api::video_records::CharacterObject;
use betterlite_http::interactions::base::Params;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Serialize;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

// the logging state both sides start from, before the call's own update
const INIT: &str = r#"{"model":"character","call_type":"fixture_call_type","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[derive(Serialize)]
pub(super) struct RecordedCall {
    function: &'static str,
    provider: String,
    timeout: f64,
    asynchronous: bool,
    api_base: Option<String>,
    api_key: Option<String>,
    extra_headers: Option<BTreeMap<String, String>>,
    name: Option<String>,
    video: Option<String>,
    character_id: Option<String>,
}

// one handler call's parts, so the trait methods build the record once
struct RecordedParts {
    function: &'static str,
    provider: String,
    timeout: Duration,
    asynchronous: bool,
    params: Params,
    extra_headers: Option<BTreeMap<String, String>>,
    name: Option<String>,
    video: Option<String>,
    character_id: Option<String>,
}

pub(super) struct Recorder {
    pub(super) logging: LoggingState,
    pub(super) calls: Vec<RecordedCall>,
    result: CharacterObject,
}

impl Recorder {
    pub(super) fn new(result: CharacterObject) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            logging: logging_state()?,
            calls: Vec::new(),
            result,
        })
    }

    fn record(&mut self, parts: RecordedParts) {
        self.calls.push(RecordedCall {
            function: parts.function,
            provider: parts.provider,
            timeout: parts.timeout.as_secs_f64(),
            asynchronous: parts.asynchronous,
            api_base: parameter_text(&parts.params, "api_base"),
            api_key: parameter_text(&parts.params, "api_key"),
            extra_headers: parts.extra_headers,
            name: parts.name,
            video: parts.video,
            character_id: parts.character_id,
        });
    }
}

impl CharacterHandler for Recorder {
    fn logging(&mut self) -> &mut LoggingState {
        &mut self.logging
    }

    fn create(
        &mut self,
        call: CreateHandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure> {
        self.record(RecordedParts {
            function: "create",
            provider: call.provider,
            timeout: call.timeout,
            asynchronous: call.is_async,
            params: call.litellm_params,
            extra_headers: call.extra_headers,
            name: Some(call.name),
            video: video_text(&call.video),
            character_id: None,
        });
        Ok(CharacterAnswered::Done(self.result.clone()))
    }

    fn get(
        &mut self,
        call: GetHandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure> {
        self.record(RecordedParts {
            function: "get",
            provider: call.provider,
            timeout: call.timeout,
            asynchronous: call.is_async,
            params: call.litellm_params,
            extra_headers: call.extra_headers,
            name: None,
            video: None,
            character_id: Some(call.character_id),
        });
        Ok(CharacterAnswered::Done(self.result.clone()))
    }
}

#[derive(Serialize)]
pub(super) struct LoggingView {
    model: String,
    call_type: String,
    user: Option<String>,
    custom_llm_provider: Option<String>,
    optional_params: IndexMap<String, Json>,
    litellm_params: IndexMap<String, Json>,
}

impl LoggingView {
    pub(super) fn of(state: &LoggingState) -> Self {
        Self {
            model: state.model.clone(),
            call_type: state.call_type.clone(),
            user: state.user.clone(),
            custom_llm_provider: state.custom_llm_provider.clone(),
            optional_params: state.optional_params.clone(),
            litellm_params: state.litellm_params.clone(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct Output {
    pub(super) result: CharacterObject,
    pub(super) calls: Vec<RecordedCall>,
    pub(super) logging: LoggingView,
}

fn video_text(video: &SuppliedMedia) -> Option<String> {
    match video {
        SuppliedMedia::Text(text) => Some(text.clone()),
        SuppliedMedia::Bytes(_) | SuppliedMedia::File(_) | SuppliedMedia::List(_) => None,
    }
}

fn parameter_text(params: &Params, name: &str) -> Option<String> {
    match params.get(name) {
        Some(JsonValue::Text(value)) => Some(value.clone()),
        None
        | Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => None,
    }
}

fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_str(INIT).or_value_error()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .or_config_error()
}
