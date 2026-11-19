use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::hosted_platform_keys::{Environment, Settings as SectionSettings};
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
use secrecy::SecretString;

use super::input::Case;

const INIT: &str = r#"{"model":"oracle","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

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
        now: DateTime::parse_from_rfc3339(NOW).or_value_error()?,
    };
    Ok((state, env))
}

pub(super) fn environment(case: &Case) -> BTreeMap<String, String> {
    case.env
        .iter()
        .filter(|(_name, value)| !value.is_empty())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// the section values a loader builds from the environment and litellm_settings
pub(super) fn section_settings(case: &Case, env: &BTreeMap<String, String>) -> SectionSettings {
    let nonempty = |value: Option<&String>| value.filter(|text| !text.is_empty()).cloned();
    SectionSettings {
        api_base: nonempty(case.globals.api_base.as_ref()),
        api_key: nonempty(case.globals.api_key.as_ref()).map(SecretString::from),
        headers: None,
        groq_key: nonempty(case.globals.groq_key.as_ref()).map(SecretString::from),
        cloudflare_api_key: nonempty(case.globals.cloudflare_api_key.as_ref())
            .map(SecretString::from),
        environment: Environment {
            groq_api_key: env.get("GROQ_API_KEY").cloned().map(SecretString::from),
            groq_api_base: env.get("GROQ_API_BASE").cloned(),
            cloudflare_api_key: env
                .get("CLOUDFLARE_API_KEY")
                .cloned()
                .map(SecretString::from),
            cloudflare_api_base: env.get("CLOUDFLARE_API_BASE").cloned(),
            cloudflare_account_id: env.get("CLOUDFLARE_ACCOUNT_ID").cloned(),
            fireworks_ai_api_key: env
                .get("FIREWORKS_AI_API_KEY")
                .cloned()
                .map(SecretString::from),
        },
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
            failure: case
                .answer
                .as_ref()
                .map(|answer| answer.status)
                .filter(|status| !(200..300).contains(status)),
        }
    }

    pub(super) fn first(&self) -> Result<CompletionCall, Box<HelperError>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .ok_or_else(|| super::input_failure(String::from("the chat arm called no handler")))
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
