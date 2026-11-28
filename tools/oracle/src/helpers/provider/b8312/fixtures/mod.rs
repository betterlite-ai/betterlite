use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_provider::hosted_platform_dispatch::call::CompletionCall;
use betterlite_provider::hosted_platform_dispatch::handler::{ChatCompletionHandler, HandlerError};
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use serde_json::{Value, json};

mod speech;

pub(super) use speech::SpeechRecorder;

const INIT: &str = r#"{"model":"oracle","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
// the two headers both sides build themselves; the HTTP client adds the rest
const KEPT: [&str; 2] = ["authorization", "content-type"];

pub(super) fn input_failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn kept<'name, Pairs>(pairs: Pairs) -> BTreeMap<String, String>
where
    Pairs: Iterator<Item = (&'name String, &'name String)>,
{
    pairs
        .filter(|(name, _)| KEPT.contains(&name.to_lowercase().as_str()))
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect()
}

// ast-grep-ignore: no-one-line-functions - every view in both files builds this same request shape
pub(super) fn request_view(url: &str, headers: &BTreeMap<String, String>) -> Value {
    json!({"url": url, "headers": headers})
}

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging(
    redactor: &Redactor,
) -> Result<(LoggingState, PrePostEnv<'_>), Box<HelperError>> {
    let init = serde_json::from_str::<LoggingInit>(INIT).or_value_error()?;
    let state = LoggingState::new(
        init,
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
        globals: GlobalFlags {
            log_raw_request_response: true,
            turn_off_message_logging: false,
        },
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").or_value_error()?,
    };
    Ok((state, env))
}

#[derive(Default)]
pub(super) struct ChatRecorder {
    calls: Mutex<Vec<CompletionCall>>,
}

impl ChatRecorder {
    pub(super) fn first(&self) -> Result<CompletionCall, Box<HelperError>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .ok_or_else(|| input_failure(String::from("the chat arm called no handler")))
    }
}

impl ChatCompletionHandler for ChatRecorder {
    type Reply = ();

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        Ok(())
    }
}
