use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::compatible_line_stream::Mode;
use betterlite_openai::compatible_wire::{NEW_ID, system_clock};
use betterlite_openai::openai_base_config::Headers;
use betterlite_provider::compatible_chat_call::error::CompatibleChatError;
use betterlite_provider::compatible_chat_call::execution::{CompatibleChatExecution, TimeoutFacts};
use betterlite_provider::compatible_chat_call::plan::ChatPlan;
use betterlite_provider::compatible_chat_call::send::{
    ChatAnswer, CompatibleChatOutcome, finish_chat,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::stream::{StreamExt as _, iter};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
// betterlite's transport default keeps a line under the size it would read from a provider
const BODY_LIMIT: usize = 16 * 1024 * 1024;
const NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    model: String,
    messages: Vec<ChatMessage>,
    custom_llm_provider: String,
    api_base: String,
    api_key: Option<String>,
    headers: Option<Headers>,
    #[serde(default)]
    optional_params: IndexMap<String, Json>,
    custom_endpoint: Option<bool>,
    timeout: Option<f64>,
    mode: Mode,
    #[serde(default)]
    fake_stream: bool,
    status: u16,
    reply: Value,
    #[serde(default)]
    watch_headers: Vec<String>,
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        input
            .calls
            .iter()
            .map(answer)
            .collect::<Result<Vec<Value>, Box<crate::helpers::HelperError>>>()
            .map(|calls| json!({"calls": calls}))
    }
}

fn invalid(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn seconds(timeout: Option<f64>) -> Result<Option<Duration>, Box<crate::helpers::HelperError>> {
    timeout
        .map(|value| Duration::try_from_secs_f64(value).map_err(|error| invalid(error.to_string())))
        .transpose()
}

fn chat_execution<'call>(
    call: &'call Call,
    key: Option<&'call SecretString>,
    timeout: Option<Duration>,
) -> CompatibleChatExecution<'call> {
    CompatibleChatExecution {
        model: &call.model,
        messages: &call.messages,
        custom_llm_provider: &call.custom_llm_provider,
        api_base: Some(&call.api_base),
        api_key: key,
        headers: call.headers.as_ref(),
        optional_params: &call.optional_params,
        custom_endpoint: call.custom_endpoint.unwrap_or(false),
        timeout,
        mode: call.mode,
        fake_stream: call.fake_stream,
        // B2230 selects the transform and B5840 the decoder; these cases carry neither
        transform_messages: None,
        streaming_decoder: None,
        identifier: NEW_ID,
        clock: system_clock,
    }
}

fn state(
    redactor: &Redactor,
    init: LoggingInit,
) -> Result<LoggingState, Box<crate::helpers::HelperError>> {
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| invalid(error.to_string()))
}

fn environment(redactor: &Redactor, now: DateTime<FixedOffset>) -> PrePostEnv<'_> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

fn timeout_value(facts: TimeoutFacts) -> Value {
    let total = facts.total.as_secs_f64();
    json!({
        "connect": facts.connect.as_secs_f64(),
        "read": total,
        "write": total,
        "pool": total,
    })
}

fn headers_value(headers: &Headers, watch: &[String]) -> Value {
    let picked: BTreeMap<String, String> = headers
        .iter()
        .filter(|(name, _)| {
            watch
                .iter()
                .any(|watched| watched.eq_ignore_ascii_case(name))
        })
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect();
    json!(picked)
}

fn request_value(
    plan: &ChatPlan<'_>,
    watch: &[String],
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let body = serde_json::to_value(&plan.body).map_err(|error| invalid(error.to_string()))?;
    Ok(json!({
        "url": &plan.url,
        "headers": headers_value(&plan.headers, watch),
        "body": body,
        "timeout": plan.request_timeout().map(timeout_value),
    }))
}

// the fixture sender answers every send with the case's status and body, as the source's client would have
fn fixture_answer(
    plan: &ChatPlan<'_>,
    call: &Call,
) -> Result<ChatAnswer, Box<crate::helpers::HelperError>> {
    let body = serde_json::to_vec(&call.reply).map_err(|error| invalid(error.to_string()))?;
    if plan.buffered() {
        return Ok(ChatAnswer::Buffered {
            status: call.status,
            body,
        });
    }
    Ok(ChatAnswer::Stream {
        body: iter([Ok::<Vec<u8>, HttpError>(body)]).boxed(),
        limit: BODY_LIMIT,
    })
}

fn outcome_value(outcome: CompatibleChatOutcome, model: &str, provider: &str) -> Value {
    match outcome {
        CompatibleChatOutcome::Reply(reply) => json!({
            "outcome": "reply",
            "reply": {"model": reply.response.model, "hidden_model": reply.hidden_model},
            "stream": null,
            "error": null,
        }),
        CompatibleChatOutcome::Stream(_) => json!({
            "outcome": "stream",
            "reply": null,
            "stream": {"model": model, "provider": provider},
            "error": null,
        }),
    }
}

fn failure(error: &CompatibleChatError, request: Option<&Value>, transport: Option<bool>) -> Value {
    json!({
        "request": request,
        "transport_stream": transport,
        "outcome": "error",
        "reply": null,
        "stream": null,
        "error": {"status": error.status_code(), "message": error.to_string()},
    })
}

fn with_send(mut fields: Value, request: Value, transport: bool) -> Value {
    if let Some(object) = fields.as_object_mut() {
        object.insert("request".to_owned(), request);
        object.insert("transport_stream".to_owned(), Value::Bool(transport));
    }
    fields
}

fn answer(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let key = call.api_key.clone().map(SecretString::from);
    let execution = chat_execution(call, key.as_ref(), seconds(call.timeout)?);
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| invalid(error.to_string()))?;
    let init =
        serde_json::from_str::<LoggingInit>(INIT).map_err(|error| invalid(error.to_string()))?;
    let now = DateTime::parse_from_rfc3339(NOW).map_err(|error| invalid(error.to_string()))?;
    let mut state = state(&redactor, init)?;
    let env = environment(&redactor, now);
    let plan = match execution.plan(&mut state, &env) {
        Ok(plan) => plan,
        Err(error) => return Ok(failure(&error, None, None)),
    };
    let request = request_value(&plan, &call.watch_headers)?;
    let transport_stream = !plan.buffered();
    let answer = fixture_answer(&plan, call)?;
    match finish_chat(&plan, answer, &mut state, &env) {
        Ok(outcome) => Ok(with_send(
            outcome_value(outcome, &call.model, &plan.reply.provider),
            request,
            transport_stream,
        )),
        Err(error) => Ok(failure(&error, Some(&request), Some(transport_stream))),
    }
}
