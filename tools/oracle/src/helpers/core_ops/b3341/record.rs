use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_character_calls::error::CharacterHandlerFailure;
use betterlite_core::api::video_character_calls::handler::{
    CharacterAnswered, CharacterHandler, HandlerArguments, HandlerCall,
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
}

impl CharacterHandler for Recorder {
    fn logging(&mut self) -> &mut LoggingState {
        &mut self.logging
    }

    fn handle(
        &mut self,
        call: HandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure> {
        let HandlerCall {
            arguments,
            provider,
            litellm_params,
            extra_headers,
            timeout,
            is_async,
        } = call;
        let (function, name, video, character_id) = match arguments {
            HandlerArguments::Create { name, video } => {
                ("create", Some(name), video_text(&video), None)
            }
            HandlerArguments::Get { character_id } => ("get", None, None, Some(character_id)),
        };
        self.calls.push(RecordedCall {
            function,
            provider,
            timeout: timeout.as_secs_f64(),
            asynchronous: is_async,
            api_base: parameter_text(&litellm_params, "api_base"),
            api_key: parameter_text(&litellm_params, "api_key"),
            extra_headers,
            name,
            video,
            character_id,
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
    let Some(JsonValue::Text(value)) = params.get(name) else {
        return None;
    };
    Some(value.clone())
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
