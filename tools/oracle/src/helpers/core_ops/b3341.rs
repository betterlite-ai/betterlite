use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_character_calls::error::{
    CharacterCallFailure, CharacterHandlerFailure,
};
use betterlite_core::api::video_character_calls::handler::{
    CharacterAnswered, CharacterHandler, CreateHandlerCall, GetHandlerCall,
};
use betterlite_core::api::video_character_calls::{
    CharacterCallDeps, CharacterOutcome, CreateCharacterCall, DEFAULT_PROVIDER,
    DEFAULT_TIMEOUT_SECONDS, GetCharacterCall, avideo_create_character, avideo_get_character,
    video_create_character, video_get_character,
};
use betterlite_core::api::video_records::{CharacterKind, CharacterObject};
use betterlite_http::interactions::base::Params;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

// B198's DEFAULT_REQUEST_TIMEOUT with REQUEST_TIMEOUT unset, the value the cases run under
const REQUEST_TIMEOUT_SECONDS: f64 = 6000.0;
const CHARACTER_ID: &str = "char-1";
const CHARACTER_CREATED_AT: i64 = 1_700_000_000;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    asynchronous: bool,
    litellm_call_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    video: Option<String>,
    #[serde(default)]
    character_id: Option<String>,
    // absent keeps the declared 600, null falls back to REQUEST_TIMEOUT and a number is the caller's
    #[serde(default, deserialize_with = "declared_timeout")]
    timeout: Option<Option<f64>>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    extra: IndexMap<String, Json>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Function {
    Create,
    Get,
}

fn declared_timeout<'de, D>(deserializer: D) -> Result<Option<Option<f64>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<f64>::deserialize(deserializer).map(Some)
}

impl Input {
    fn provider(&self) -> String {
        self.custom_llm_provider
            .clone()
            .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned())
    }

    fn timeout(&self) -> Option<f64> {
        match self.timeout {
            None => Some(DEFAULT_TIMEOUT_SECONDS),
            Some(value) => value,
        }
    }

    fn create_call(&self) -> CreateCharacterCall {
        CreateCharacterCall {
            name: self.name.clone().unwrap_or_default(),
            video: SuppliedMedia::Text(self.video.clone().unwrap_or_default()),
            timeout: self.timeout(),
            custom_llm_provider: self.provider(),
            extra_headers: self.extra_headers.clone(),
            litellm_call_id: Some(self.litellm_call_id.clone()),
            extra: self.extra.clone(),
            ..CreateCharacterCall::default()
        }
    }

    fn get_call(&self) -> GetCharacterCall {
        GetCharacterCall {
            character_id: self.character_id.clone().unwrap_or_default(),
            timeout: self.timeout(),
            custom_llm_provider: self.provider(),
            extra_headers: self.extra_headers.clone(),
            litellm_call_id: Some(self.litellm_call_id.clone()),
            extra: self.extra.clone(),
            ..GetCharacterCall::default()
        }
    }

    // the object the fixture handler answers with, named by the create call or the lookup's fixed name
    fn answered_object(&self) -> CharacterObject {
        let name = match self.function {
            Function::Create => self.name.clone().unwrap_or_default(),
            Function::Get => "Wren".to_owned(),
        };
        CharacterObject {
            id: CHARACTER_ID.to_owned(),
            object: CharacterKind::Character,
            created_at: CHARACTER_CREATED_AT,
            name,
            hidden_params: IndexMap::new(),
        }
    }
}

#[derive(Serialize)]
struct RecordedCall {
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

struct Recorder {
    logging: LoggingState,
    calls: Vec<RecordedCall>,
    result: CharacterObject,
}

impl Recorder {
    fn new(result: CharacterObject) -> Self {
        Self {
            logging: logging_state(),
            calls: Vec::new(),
            result,
        }
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
struct LoggingView {
    model: String,
    call_type: String,
    user: Option<String>,
    custom_llm_provider: Option<String>,
    optional_params: IndexMap<String, Json>,
    litellm_params: IndexMap<String, Json>,
}

impl LoggingView {
    fn of(state: &LoggingState) -> Self {
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
struct Output {
    result: CharacterObject,
    calls: Vec<RecordedCall>,
    logging: LoggingView,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(answer(input))
    }
}

async fn answer(input: &Input) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let mut recorder = Recorder::new(input.answered_object());
    let deps = CharacterCallDeps {
        redactor: &redactor,
        request_timeout_seconds: REQUEST_TIMEOUT_SECONDS,
    };
    let character = match (input.function, input.asynchronous) {
        (Function::Create, false) => direct(video_create_character(
            &mut recorder,
            deps,
            &input.create_call(),
        ))?,
        (Function::Create, true) => {
            avideo_create_character(&mut recorder, deps, &input.create_call())
                .await
                .map_err(|error| failure(&error))?
        }
        (Function::Get, false) => {
            direct(video_get_character(&mut recorder, deps, &input.get_call()))?
        }
        (Function::Get, true) => avideo_get_character(&mut recorder, deps, &input.get_call())
            .await
            .map_err(|error| failure(&error))?,
    };
    let output = Output {
        result: character,
        calls: recorder.calls,
        logging: LoggingView::of(&recorder.logging),
    };
    serde_json::to_value(output).or_value_error()
}

// the sync answer is in hand, or the handler raised a future this operation waits out
fn direct(
    outcome: Result<CharacterOutcome<'_>, CharacterCallFailure>,
) -> Result<CharacterObject, Box<HelperError>> {
    match outcome.map_err(|error| failure(&error))? {
        CharacterOutcome::Done(character) => Ok(character),
        CharacterOutcome::Pending(pending) => {
            futures_executor::block_on(pending).map_err(|error| failure(&error))
        }
    }
}

fn failure(error: &CharacterCallFailure) -> Box<HelperError> {
    Box::new(HelperError {
        class: "CharacterCallFailure".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
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
        | Some(JsonValue::Null)
        | Some(JsonValue::Bool(_))
        | Some(JsonValue::Number(_))
        | Some(JsonValue::Array(_))
        | Some(JsonValue::Object(_)) => None,
    }
}

fn logging_state() -> LoggingState {
    let start_time =
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339");
    LoggingState::new(
        LoggingInit {
            model: "character".to_owned(),
            call_type: "fixture_call_type".to_owned(),
            start_time,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the fixture redactor builds"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}
