use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::self_hosted_dispatch::Settings as SectionSettings;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_provider::self_hosted_dispatch::call::CompletionCall;
use betterlite_provider::self_hosted_dispatch::handler::{ChatCompletionHandler, HandlerError};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use super::input::{Answer, Case};

// the placeholder a case uses for the host it cannot know when it is written
pub(super) const ORACLE_UPSTREAM: &str = "https://self-hosted.example.com";
const INIT: &str = r#"{"model":"oracle","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW_SECONDS: i64 = 1_700_000_005;

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
        now: DateTime::<FixedOffset>::from(
            DateTime::from_timestamp(NOW_SECONDS, 0).unwrap_or_default(),
        ),
    };
    Ok((state, env))
}

pub(super) fn environment(case: &Case) -> BTreeMap<String, String> {
    case.env
        .iter()
        .map(|(name, value)| (name.clone(), value.replace("{{upstream}}", ORACLE_UPSTREAM)))
        .collect()
}

// the section values a loader builds from the environment and litellm_settings
pub(super) fn section_settings(case: &Case, env: &BTreeMap<String, String>) -> SectionSettings {
    SectionSettings {
        hosted_vllm_api_base: env.get("HOSTED_VLLM_API_BASE").cloned(),
        hosted_vllm_api_key: env
            .get("HOSTED_VLLM_API_KEY")
            .cloned()
            .map(SecretString::from),
        ollama_api_base: env.get("OLLAMA_API_BASE").cloned(),
        ollama_api_key: env.get("OLLAMA_API_KEY").cloned().map(SecretString::from),
        lemonade_api_key: env.get("LEMONADE_API_KEY").cloned().map(SecretString::from),
        api_base: case.globals.api_base.clone(),
        api_key: case.globals.api_key.as_deref().map(SecretString::from),
        ollama_key: case.globals.ollama_key.as_deref().map(SecretString::from),
        lemonade_key: case.globals.lemonade_key.as_deref().map(SecretString::from),
    }
}

pub(super) struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl Recorder {
    pub(super) fn for_case(case: &Case) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            // litellm's handler succeeds on a 2xx answer and raises on any other status
            failure: case
                .answer
                .as_ref()
                .map(|answer| answer.status)
                .filter(|status| !(200..300).contains(status)),
        }
    }

    pub(super) fn first(&self) -> Result<CompletionCall, Box<HelperError>> {
        let recorded = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        recorded.first().map_or_else(
            || {
                Err(super::input_failure(String::from(
                    "the chat arm called no handler",
                )))
            },
            |call| Ok(call.clone()),
        )
    }
}

impl ChatCompletionHandler for Recorder {
    type Reply = String;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        self.failure.map_or_else(
            || Ok(String::from("reply")),
            |status_code| Err(HandlerError::Upstream { status_code }),
        )
    }
}

pub(super) fn answer(case: &Case) -> Result<&Answer, Box<HelperError>> {
    case.answer
        .as_ref()
        .ok_or_else(|| super::input_failure(String::from("the case names no upstream answer")))
}
