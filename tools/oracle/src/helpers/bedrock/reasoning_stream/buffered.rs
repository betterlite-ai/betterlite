use std::time::Duration;

use betterlite_bedrock::reasoning_stream::buffered::{
    ReplyInput, RuntimeReply, transform_response,
};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_openai::compatible_wire::{LogInput, NEW_ID, ReplyOptions, system_clock};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

const INIT: &str = r#"{"model":"openai.gpt-oss-20b-1:0","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

struct QuietSink;

impl LogSink for QuietSink {
    fn emit(&self, _record: LogRecord) {}
}

fn logging_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).or_value_error()?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .or_value_error()
}

fn reply_env<'env>(
    redactor: &'env Redactor,
    sink: &'env QuietSink,
) -> Result<PrePostEnv<'env>, Box<HelperError>> {
    Ok(PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").or_value_error()?,
    })
}

fn reply_view(split: &RuntimeReply, names: &IndexMap<String, String>) -> Value {
    let choices: Vec<Value> = split
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": &choice.finish_reason,
                "content": &choice.content,
                "reasoning_content": &choice.reasoning_content,
            })
        })
        .collect();
    let headers: IndexMap<&str, &str> = names
        .keys()
        .filter_map(|name| {
            split
                .headers
                .headers
                .get(name)
                .map(|value| (name.as_str(), value.as_str()))
        })
        .collect();
    json!({
        "id": split.response.id.as_str(),
        "created": split.response.created,
        "choices": choices,
        "headers": headers,
    })
}

pub(super) fn run(reply: &Value, headers: &IndexMap<String, String>) -> HelperResult {
    let bytes = serde_json::to_vec(reply).or_value_error()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let mut state = logging_state(&redactor)?;
    let sink = QuietSink;
    let env = reply_env(&redactor, &sink)?;
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: NEW_ID,
        clock: system_clock,
    };
    let input = LogInput {
        messages: Json::List(Vec::new()),
        data: RawBody::Object(IndexMap::new()),
    };
    let existing = AdditionalHeaders::new();
    let split = transform_response(
        &bytes,
        &mut state,
        &env,
        input,
        &ReplyInput {
            options,
            provider_headers: headers,
            existing_additional_headers: &existing,
        },
    )
    .or_value_error()?;
    Ok(reply_view(&split, headers))
}
