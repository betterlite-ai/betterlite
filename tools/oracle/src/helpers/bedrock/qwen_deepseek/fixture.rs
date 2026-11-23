use std::time::Duration;

use betterlite_bedrock::qwen_deepseek::reply::{Choice, Reply};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_redact::secret::Redactor;
use chrono::DateTime;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

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
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").or_value_error()?,
    };
    Ok((state, env))
}

// The reply both transforms receive: the Choices constructor has already mapped each seeded finish reason.
pub(super) fn build_reply(seed: &Reply) -> Reply {
    Reply {
        id: seed.id.clone(),
        object: seed.object.clone(),
        created: seed.created,
        model: seed.model.clone(),
        choices: seed.choices.iter().map(mapped_choice).collect(),
        usage: seed.usage.clone(),
    }
}

fn mapped_choice(choice: &Choice) -> Choice {
    let mapped = map_finish_reason(choice.finish_reason.as_deref().unwrap_or("stop"));
    Choice {
        index: choice.index,
        message: choice.message.clone(),
        finish_reason: Some(mapped.reason.as_str().to_owned()),
        provider_specific_fields: choice.provider_specific_fields.clone(),
    }
}
