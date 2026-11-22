use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};

use betterlite_oracle_helper_api::error::HelperError;

use super::ResponseSeed;
use super::error::value_error;

const INIT: &str = r#"{"model":"fixture-model","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

pub(super) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging<'borrow>(
    redactor: &'borrow Redactor,
    sink: &'borrow Silent,
) -> Result<(LoggingState, PrePostEnv<'borrow>), Box<HelperError>> {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT)
            .map_err(|error| value_error(error.to_string()))?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| value_error(error.to_string()))?;
    let env = PrePostEnv {
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
        now: DateTime::<FixedOffset>::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| value_error(error.to_string()))?,
    };
    Ok((state, env))
}

pub(super) fn build_response(seed: &ResponseSeed) -> ChatResponse {
    let choices = seed
        .choices
        .iter()
        .map(|choice| ChatChoice {
            index: choice.index,
            message: AssistantMessage {
                role: choice.message.role.clone(),
                content: choice.message.content.clone(),
                tool_calls: choice.message.tool_calls.as_ref().map(|_calls| Vec::new()),
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: choice.finish_reason.clone(),
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        })
        .collect();
    ChatResponse {
        id: seed.id.clone(),
        object: String::from("chat.completion"),
        created: 0,
        model: String::new(),
        choices,
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}
