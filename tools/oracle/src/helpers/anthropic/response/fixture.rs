use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_anthropic::response::error::ResponseError;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{Logging, RawResponse, ResponseRequest, transform_response};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::helpers::{HelperCall, invoke};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use indexmap::IndexMap;
use serde_json::json;
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::Input;
use super::state;

const INIT: &str = r#"{"model":"claude-sonnet-4-6","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","messages":[],"litellm_params":{},"request_logging":{}}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("a fixture header name is not valid")]
    HeaderName {
        #[source]
        source: InvalidHeaderName,
    },
    #[error("a fixture header value is not valid")]
    HeaderValue {
        #[source]
        source: InvalidHeaderValue,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("the token_count helper failed: {message}")]
struct TokenCountFailure {
    message: String,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let headers = header_map(input).or_config_error()?;
    let warned = Warned::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let silent = Silent;
    let mut logging = logging_state(&redactor)?;
    let env = logging_env(&redactor, &silent)?;
    let request = request_of(input);
    let raw = raw_of(input, &headers);
    let mut envelope = ResponseEnvelope::new(input.id.clone());
    let subscriber = registry().with(warned.clone());
    let boxed = with_default(subscriber, || {
        transform_response(
            &mut envelope,
            &raw,
            &request,
            Logging {
                state: &mut logging,
                env: &env,
            },
            &TokenizerCounter,
        )
        .map_err(Box::new)
    });
    let result = boxed.map_err(|error| *error);
    let details = &logging.model_call_details;
    let fired = warned.0.load(Ordering::Relaxed);
    outcome(result, &envelope, details, fired)
}

fn request_of(input: &Input) -> ResponseRequest<'_> {
    ResponseRequest {
        messages: &input.messages,
        optional_params: &input.optional_params,
        litellm_params: &input.litellm_params,
        request_data: &input.request_data,
        json_mode: input.json_mode,
        now: input.now,
        disable_add_prefix_to_prompt: input.disable_add_prefix_to_prompt,
    }
}

const fn raw_of<'raw>(input: &'raw Input, headers: &'raw HeaderMap) -> RawResponse<'raw> {
    RawResponse {
        status: input.status,
        headers,
        body: input.raw_text.as_bytes(),
    }
}

fn outcome(
    result: Result<(), ResponseError<TokenCountFailure>>,
    envelope: &ResponseEnvelope,
    details: &IndexMap<String, Json>,
    fired: bool,
) -> HelperResult {
    match result {
        Ok(()) => Ok(state::reply(envelope, details, fired)),
        Err(ResponseError::Anthropic {
            status,
            headers: error_headers,
            ..
        }) => Ok(state::anthropic_error(
            status,
            &error_headers,
            details,
            fired,
        )),
        Err(error) => Err(Box::new(HelperError {
            class: "ResponseError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}

fn header_map(input: &Input) -> Result<HeaderMap, FixtureError> {
    let mut headers = HeaderMap::new();
    for (name, value) in &input.headers {
        headers.append(
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|source| FixtureError::HeaderName { source })?,
            HeaderValue::from_str(value).map_err(|source| FixtureError::HeaderValue { source })?,
        );
    }
    // httpx adds content-length to a response built from a non-empty body and leaves an empty one bare
    if !input.raw_text.is_empty() && !headers.contains_key("content-length") {
        headers.insert(
            HeaderName::from_static("content-length"),
            HeaderValue::from_str(&input.raw_text.len().to_string())
                .map_err(|source| FixtureError::HeaderValue { source })?,
        );
    }
    Ok(headers)
}

fn logging_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    let init = serde_json::from_str::<LoggingInit>(INIT).or_config_error()?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .or_config_error()
}

fn logging_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Silent,
) -> Result<PrePostEnv<'env>, Box<HelperError>> {
    let now = DateTime::<FixedOffset>::parse_from_rfc3339(NOW).or_config_error()?;
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
        now,
    })
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

#[derive(Clone, Default)]
struct Warned(Arc<AtomicBool>);

impl<Inner: Subscriber> Layer<Inner> for Warned {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == Level::WARN {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}

// the tokenizer helper owns the token_counter setup, so the fallback estimate goes through that one operation
struct TokenizerCounter;

impl ReasoningTokenCounter for TokenizerCounter {
    type Failure = TokenCountFailure;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Self::Failure> {
        let call = HelperCall {
            operation: "token_count".to_owned(),
            input: json!({
                "function": "token_counter",
                "model": "",
                "text": text,
                "count_response_tokens": true,
            }),
        };
        let output = invoke(&[betterlite_oracle_tokenizer::GROUP], &call);
        if let Some(error) = output.error {
            return Err(TokenCountFailure {
                message: error.message,
            });
        }
        output
            .value
            .and_then(|value| value.as_u64())
            .and_then(|count| usize::try_from(count).ok())
            .ok_or_else(|| TokenCountFailure {
                message: "the token_count helper returned no count".to_owned(),
            })
    }
}
