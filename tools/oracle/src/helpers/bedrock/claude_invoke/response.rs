use std::time::Duration;

use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{Logging, RawResponse, ResponseRequest, transform_response};
use betterlite_bedrock::claude_invoke::response::transform_response as claude_transform_response;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;

use super::mock::{NoReasoning, Silent};

const INIT: &str = r#"{"model":"claude-plain","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","messages":[],"litellm_params":{},"request_logging":{}}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

fn log_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).or_config_error()?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .or_config_error()?;
    Ok(state)
}

fn pre_post_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Silent,
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
        now: DateTime::<FixedOffset>::parse_from_rfc3339(NOW).or_config_error()?,
    })
}

pub(super) fn response(
    raw: &str,
    messages: &[Json],
    optional_params: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Json>,
    now: i64,
) -> Result<ResponseEnvelope, Box<HelperError>> {
    let headers = HeaderMap::new();
    let raw_response = RawResponse {
        status: 200,
        headers: &headers,
        body: raw.as_bytes(),
    };
    let request_data = IndexMap::new();
    let request = ResponseRequest {
        messages,
        optional_params,
        litellm_params,
        request_data: &request_data,
        json_mode: None,
        now,
        disable_add_prefix_to_prompt: false,
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let mut state = log_state(&redactor)?;
    let sink = Silent;
    let env = pre_post_env(&redactor, &sink)?;
    let mut envelope = ResponseEnvelope::new("generated".to_owned());
    claude_transform_response(
        &mut envelope,
        &raw_response,
        &request,
        Logging {
            state: &mut state,
            env: &env,
        },
        &NoReasoning,
    )
    .or_config_error()?;
    Ok(envelope)
}
