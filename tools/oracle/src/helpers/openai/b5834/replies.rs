use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::perplexity_embedding::error::PerplexityEmbeddingError;
use betterlite_openai::perplexity_embedding::reply::transform_embedding_response;
use betterlite_openai::perplexity_responses::error::PerplexityResponsesError;
use betterlite_openai::perplexity_responses::transform_response;
use betterlite_openai::response_events::{RawResponse, ReplyBody, ResponsesReply};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::ReplyCall;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const INIT: &str = r#"{"model":"sonar","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";
const CREATED: i64 = 1_767_225_600;

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

// b6412's Python view: the request model is "m", and usage keeps the two token counts
pub(super) fn embedding_reply(call: &ReplyCall) -> Value {
    match transform_embedding_response("m", call.status, call.body.as_bytes()) {
        Ok(reply) => json!({
            "ok": true,
            "value": {
                "model": reply.model,
                "data": reply.data,
                "object": reply.object,
                "usage": {
                    "prompt_tokens": reply.usage.prompt_tokens,
                    "total_tokens": reply.usage.total_tokens,
                },
            },
        }),
        Err(PerplexityEmbeddingError::Provider { error }) => json!({"raised": error.status_code}),
        Err(
            PerplexityEmbeddingError::TooLarge { .. }
            | PerplexityEmbeddingError::Shape { .. }
            | PerplexityEmbeddingError::Base64 { .. }
            | PerplexityEmbeddingError::Encode { .. },
        ) => json!({"raised": null}),
    }
}

pub(super) fn responses_replies(calls: &[ReplyCall]) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let now = DateTime::parse_from_rfc3339(NOW).or_value_error()?;
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        values.push(one_reply(call, &redactor, now)?);
    }
    Ok(Value::Array(values))
}

// LiteLLM builds a logging state per call, so each reply gets its own
fn one_reply(
    call: &ReplyCall,
    redactor: &Redactor,
    now: DateTime<FixedOffset>,
) -> Result<Value, Box<HelperError>> {
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).or_value_error()?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .or_value_error()?;
    let env = PrePostEnv {
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
    };
    let headers: IndexMap<String, String> = call
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    let reply = RawResponse {
        status: call.status,
        headers: &headers,
        text: &call.body,
    };
    Ok(reply_value(transform_response(
        &reply, CREATED, &mut state, &env,
    )))
}

fn reply_value(outcome: Result<ResponsesReply, PerplexityResponsesError>) -> Value {
    match outcome {
        Ok(transformed) => match transformed.response {
            ReplyBody::Typed(response) => json!({"ok": true, "id": response.id}),
            ReplyBody::Lax(fields) => json!({"ok": true, "id": fields.get("id")}),
        },
        Err(PerplexityResponsesError::Failed { error }) => json!({"raised": error.status_code}),
        Err(PerplexityResponsesError::Input(_) | PerplexityResponsesError::Reply(_)) => {
            json!({"raised": null})
        }
    }
}
