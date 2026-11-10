use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::env::PrePostEnv;
use super::error::PrePostError;
use crate::exporter_options::redaction::REDACTED_TEXT;
use crate::logging_privacy::request::Decision;
use crate::logging_privacy::snapshot::callback_snapshot;
use crate::logging_state::LoggingState;
use crate::python_json;

pub struct PreCallEvent<'call> {
    pub model: &'call str,
    pub messages: &'call Json,
    pub details: &'call Json,
}

pub struct PostCallEvent<'call> {
    pub details: &'call Json,
    pub start_time: DateTime<FixedOffset>,
}

pub trait ApiCallLogger {
    fn log_pre_api_call(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError>;

    fn log_post_api_call(&self, event: &PostCallEvent<'_>) -> Result<(), PrePostError>;
}

// Callable input callbacks and the supabase client, whose exporters belong to B110-B118
pub trait InputEventSink {
    fn log_input_event(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError>;
}

pub trait SentrySink {
    fn add_breadcrumb(&self, message: &str) -> Result<(), PrePostError>;

    fn capture_exception(&self, message: &str);
}

#[derive(Clone, Copy)]
pub enum InputCallback<'hook> {
    Sentry,
    Logger(&'hook dyn ApiCallLogger),
    InputEvent(&'hook dyn InputEventSink),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pre,
    Post,
}

const CONTENT_KEYS: [&str; 3] = ["messages", "input", "prompt"];

impl Phase {
    const fn label(self) -> &'static str {
        match self {
            Self::Pre => "pre-call",
            Self::Post => "post-call",
        }
    }

    const fn failure_headline(self) -> &'static str {
        match self {
            Self::Pre => "litellm.Logging.pre_call(): Exception occurred -",
            Self::Post => {
                "LiteLLM.LoggingError: [Non-Blocking] Exception occurred while post-call logging with integrations"
            }
        }
    }
}

fn breadcrumb_text(phase: Phase, details: &Json, env: &PrePostEnv<'_>) -> String {
    let mut shown = details.clone();
    if let Json::Object(fields) = &mut shown
        && env.globals.turn_off_message_logging
    {
        fields.retain(|name, _| !CONTENT_KEYS.contains(&name.as_str()));
    }
    format!(
        "Model Call Details {}: {}",
        phase.label(),
        python_json::dumps(&shown)
    )
}

fn run_one(
    callback: InputCallback<'_>,
    phase: Phase,
    state: &LoggingState,
    details: &Json,
    env: &PrePostEnv<'_>,
) -> Result<(), PrePostError> {
    let mut snapshot = callback_snapshot(
        Json::Object(IndexMap::from([(
            "messages".to_owned(),
            state.messages.clone().map_or(Json::Null, Json::List),
        )])),
        env.decision,
        env.redactor,
    );
    let shown_messages = match &mut snapshot {
        Json::Object(fields) => fields.shift_remove("messages").unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    };
    let pre = PreCallEvent {
        model: &state.model,
        messages: &shown_messages,
        details,
    };
    match (callback, phase) {
        (InputCallback::Sentry, Phase::Pre | Phase::Post) => env.sentry.map_or(Ok(()), |sentry| {
            sentry.add_breadcrumb(&breadcrumb_text(phase, details, env))
        }),
        (InputCallback::Logger(logger), Phase::Pre) => logger.log_pre_api_call(&pre),
        (InputCallback::Logger(logger), Phase::Post) => logger.log_post_api_call(&PostCallEvent {
            details,
            start_time: state.start_time,
        }),
        (InputCallback::InputEvent(sink), Phase::Pre) => sink.log_input_event(&pre),
        (InputCallback::InputEvent(_), Phase::Post) => Ok(()),
    }
}

// redact_tree only rewrites a key named response, so the call fields it leaves alone are replaced here
pub fn redact_callback_fields(snapshot: &mut Json, decision: Decision) {
    if !decision.content {
        return;
    }
    let Json::Object(fields) = snapshot else {
        return;
    };
    if let Some(response) = fields.get_mut("original_response") {
        *response = Json::Text(REDACTED_TEXT.to_owned());
    }
    let Some(Json::Object(call)) = fields.get_mut("additional_args") else {
        return;
    };
    if let Some(body) = call.get_mut("complete_input_dict")
        && matches!(body, Json::Text(_))
    {
        *body = Json::Text(REDACTED_TEXT.to_owned());
    }
    if let Some(request_str) = call.get_mut("request_str") {
        *request_str = Json::Text(REDACTED_TEXT.to_owned());
    }
}

pub fn run_input_callbacks(state: &LoggingState, phase: Phase, env: &PrePostEnv<'_>) {
    let mut details = callback_snapshot(
        Json::Object(state.model_call_details.clone()),
        env.decision,
        env.redactor,
    );
    redact_callback_fields(&mut details, env.decision);
    for callback in env.input_callbacks {
        if let Err(error) = run_one(*callback, phase, state, &details, env) {
            env.report_failure(phase.failure_headline(), &error);
        }
    }
}
