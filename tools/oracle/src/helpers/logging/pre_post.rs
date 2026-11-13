use crate::compare::helper::HelperError;
use crate::helpers::{Helper, HelperResult};
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogLevel, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::hooks::{
    ApiCallLogger, InputCallback, PostCallEvent, PreCallEvent,
};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use std::cell::RefCell;
use std::time::Duration;

pub(crate) struct Op;

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    #[default]
    Pre,
    Post,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct DebugFlags {
    verbose: bool,
    request_debug: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CaptureFlags {
    log_raw_request_response: bool,
    raw_request_only: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct FailureFlags {
    failing_callback: bool,
    failing_logger_fn: bool,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    action: Action,
    #[serde(default = "default_call_type")]
    call_type: String,
    messages: Option<Json>,
    #[serde(default)]
    litellm_params: IndexMap<String, Json>,
    #[serde(flatten)]
    debug: DebugFlags,
    #[serde(flatten)]
    capture: CaptureFlags,
    #[serde(flatten)]
    failures: FailureFlags,
    #[serde(default)]
    turn_off_message_logging: bool,
    #[serde(rename = "input", default = "default_input")]
    payload: Json,
    model: Option<String>,
    #[serde(default = "default_original_response")]
    original_response: Json,
    #[serde(default)]
    additional_args: AdditionalArgs,
}

fn default_call_type() -> String {
    "completion".to_owned()
}

const fn default_input() -> Json {
    Json::Null
}

const fn default_original_response() -> Json {
    Json::Null
}

struct Recorder {
    events: RefCell<Vec<Value>>,
    failing: bool,
}

impl Recorder {
    fn finish(&self, entry: Value) -> Result<(), PrePostError> {
        self.events.borrow_mut().push(entry);
        if self.failing {
            return Err(PrePostError::CallbackFailed {
                message: "callback exploded".to_owned(),
            });
        }
        Ok(())
    }
}

fn detail(details: &Json, name: &str) -> Json {
    match details {
        Json::Object(fields) => fields.get(name).cloned().unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    }
}

impl LogSink for Recorder {
    fn emit(&self, record: LogRecord) {
        let level = match record.level {
            LogLevel::Debug => "debug",
            LogLevel::Warning => "warning",
            LogLevel::Error => "error",
        };
        self.events.borrow_mut().push(serde_json::json!({
            "kind": "log",
            "level": level,
            "message": record.message,
        }));
    }
}

impl ApiCallLogger for Recorder {
    fn log_pre_api_call(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        let kind = detail(event.details, "log_event_type");
        self.finish(serde_json::json!({
            "kind": "logger_pre",
            "model": event.model,
            "messages": event.messages,
            "log_event_type": kind,
        }))
    }

    fn log_post_api_call(&self, event: &PostCallEvent<'_>) -> Result<(), PrePostError> {
        self.finish(serde_json::json!({
            "kind": "logger_post",
            "log_event_type": detail(event.details, "log_event_type"),
            "start_time": event.start_time.to_rfc3339(),
        }))
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let scrubber =
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(error.to_string()))?;
        let mut state = build_state(input, &scrubber)?;
        state.litellm_request_debug = input.debug.request_debug;
        let recorder = Recorder {
            events: RefCell::new(Vec::new()),
            failing: input.failures.failing_callback,
        };
        let error = dispatch(input, &mut state, &recorder, &scrubber)?;
        Ok(output(&state, &recorder, &error))
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "PrePostError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn build_state(input: &Input, redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_value(serde_json::json!({
        "model": "gpt-4o",
        "call_type": input.call_type,
        "start_time": "2026-01-01T00:00:00+00:00",
        "litellm_call_id": "call-1",
        "function_id": "fn-1",
        "litellm_trace_id": "trace-1",
        "messages": input.messages,
        "litellm_params": input.litellm_params,
        "request_logging": {
            "log_raw_request_response": input.capture.log_raw_request_response,
            "raw_request_only": input.capture.raw_request_only,
        },
    }))
    .map_err(|error| failure(error.to_string()))?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .map_err(|error| failure(error.to_string()))
}

fn dispatch(
    input: &Input,
    state: &mut LoggingState,
    recorder: &Recorder,
    scrubber: &Redactor,
) -> Result<Value, Box<HelperError>> {
    let callbacks = [InputCallback::Logger(recorder)];
    let failing_logger_fn = input.failures.failing_logger_fn;
    let logger_fn = |details: &Json| {
        recorder.events.borrow_mut().push(serde_json::json!({
            "kind": "logger_fn",
            "log_event_type": detail(details, "log_event_type"),
            "input": detail(details, "input"),
        }));
        if failing_logger_fn {
            return Err(PrePostError::CallbackFailed {
                message: "logger_fn exploded".to_owned(),
            });
        }
        Ok(())
    };
    let env = PrePostEnv {
        debug: DebugSettings {
            verbose: input.debug.verbose,
            json_logs: false,
        },
        globals: GlobalFlags {
            log_raw_request_response: false,
            turn_off_message_logging: input.turn_off_message_logging,
        },
        decision: Decision {
            content: input.turn_off_message_logging,
            key_info: false,
        },
        redactor: scrubber,
        log: recorder,
        input_callbacks: &callbacks,
        sentry: None,
        logger_fn: Some(&logger_fn),
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| failure(error.to_string()))?,
    };
    Ok(invoke(input, state, &env))
}

fn invoke(input: &Input, state: &mut LoggingState, env: &PrePostEnv<'_>) -> Value {
    if matches!(input.action, Action::Post) {
        let call = PostCall {
            original_response: input.original_response.clone(),
            input: input.payload.clone(),
            additional_args: input.additional_args.clone(),
        };
        post_call(state, &call, env);
        return Value::Null;
    }
    let call = PreCall {
        input: input.payload.clone(),
        model: input.model.clone(),
        additional_args: input.additional_args.clone(),
    };
    pre_call(state, &call, env).map_or_else(
        |error| serde_json::json!({"class": "RawRequestCaptured", "message": error.to_string()}),
        |_| Value::Null,
    )
}

fn output(state: &LoggingState, recorder: &Recorder, error: &Value) -> Value {
    let details = &state.model_call_details;
    let set = |name: &str| !matches!(details.get(name), None | Some(Json::Null));
    let events = recorder.events.borrow().clone();
    serde_json::json!({
        "events": events,
        "details": {
            "log_event_type": details.get("log_event_type"),
            "input": details.get("input"),
            "model": details.get("model"),
            "original_response": details.get("original_response"),
            "api_call_start_time_set": set("api_call_start_time"),
            "first_api_call_start_time_set": set("first_api_call_start_time"),
            "raw_request_typed_dict": details.get("raw_request_typed_dict"),
        },
        "litellm_params": {
            "api_base": state.litellm_params.get("api_base"),
            "metadata": state.litellm_params.get("metadata"),
        },
        "error": error,
    })
}
