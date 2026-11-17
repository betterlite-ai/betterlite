use std::fmt::Display;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::converse_usage::transform::ReasoningTokenCounter;
use betterlite_bedrock::error::BedrockError as MetadataError;
use betterlite_bedrock::request_policy::{BedrockRequestPolicy, Params, RequestValue};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::{Value, json};

use betterlite_oracle_runner::helpers::{HelperCall, invoke};

const INIT: &str = r#"{"model":"fixture-model","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

pub(crate) fn failure<Failure>(error: Failure) -> Box<HelperError>
where
    Failure: Display,
{
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) fn status_failure(error: &MetadataError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("BedrockError"),
        status: Some(error.status_code()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) fn fixture(
    redactor: &Redactor,
) -> Result<(LoggingState, PrePostEnv<'_>), Box<HelperError>> {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).map_err(failure)?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(failure)?;
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
        now: DateTime::<FixedOffset>::parse_from_rfc3339(NOW).map_err(failure)?,
    };
    Ok((state, env))
}

pub(crate) fn log_view(state: &LoggingState) -> Value {
    let details = &state.model_call_details;
    json!({
        "input": details.get("input").map_or(Value::Null, |value| json!(value)),
        "original_response": details.get("original_response").map_or(Value::Null, |value| json!(value)),
        "additional_args": details.get("additional_args").map_or(Value::Null, |value| json!(value)),
        "log_event_type": details.get("log_event_type").map_or(Value::Null, |value| json!(value)),
    })
}

// The reply clock is injected as whole seconds, so the fixture never reads the wall clock.
pub(crate) fn instant(now: i64) -> Result<SystemTime, Box<HelperError>> {
    let magnitude = Duration::from_secs(now.unsigned_abs());
    let instant = if now < 0 {
        UNIX_EPOCH.checked_sub(magnitude)
    } else {
        UNIX_EPOCH.checked_add(magnitude)
    };
    instant.ok_or_else(|| failure("the injected clock is out of range"))
}

pub(crate) fn request_value(value: &Value) -> RequestValue {
    if let Some(text) = value.as_str() {
        return RequestValue::Text(text.to_owned());
    }
    let Some(object) = value.as_object() else {
        return RequestValue::Raw(value.to_string());
    };
    RequestValue::Mapping(
        object
            .iter()
            .map(|(key, entry)| (key.clone(), request_value(entry)))
            .collect(),
    )
}

pub(crate) fn entries(value: &Value) -> Params {
    match request_value(value) {
        RequestValue::Mapping(pairs) => pairs,
        RequestValue::Text(_) | RequestValue::Secret(_) | RequestValue::Raw(_) => Params::new(),
    }
}

pub(crate) struct EnvironmentSetup {
    pub(crate) headers: IndexMap<String, String>,
    pub(crate) key: Option<SecretString>,
    pub(crate) policy: Option<BedrockRequestPolicy>,
    pub(crate) params: Params,
}

// The metadata policy exists only when the case opts into fields, and the key stays secret until the merge formats it.
pub(crate) fn environment_setup(
    fields: &[String],
    headers: &[(String, String)],
    litellm_params: &Value,
    api_key: Option<&str>,
) -> EnvironmentSetup {
    let policy = (!fields.is_empty()).then(|| BedrockRequestPolicy {
        fields: fields.iter().cloned().collect(),
    });
    EnvironmentSetup {
        headers: headers.iter().cloned().collect(),
        key: api_key.map(SecretString::from),
        policy,
        params: entries(litellm_params),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the token_count helper failed: {message}")]
pub(crate) struct TokenCountFailure {
    pub(crate) message: String,
}

#[derive(Serialize)]
struct TokenCountRequest<'text> {
    function: &'static str,
    model: &'static str,
    text: &'text str,
    count_response_tokens: bool,
}

pub(crate) struct TokenizerCounter;

impl ReasoningTokenCounter for TokenizerCounter {
    type Failure = TokenCountFailure;

    fn count_reasoning_tokens(&self, text: &str) -> Result<i64, Self::Failure> {
        let request = TokenCountRequest {
            function: "token_counter",
            model: "",
            text,
            count_response_tokens: true,
        };
        let input = serde_json::to_value(request).map_err(|error| TokenCountFailure {
            message: error.to_string(),
        })?;
        let output = invoke(
            &[betterlite_oracle_tokenizer::GROUP],
            &HelperCall {
                operation: String::from("token_count"),
                input,
            },
        );
        if let Some(error) = output.error {
            return Err(TokenCountFailure {
                message: error.message,
            });
        }
        let Some(value) = output.value else {
            return Err(TokenCountFailure {
                message: String::from("the token_count helper returned no count"),
            });
        };
        serde_json::from_value(value).map_err(|error| TokenCountFailure {
            message: error.to_string(),
        })
    }
}
