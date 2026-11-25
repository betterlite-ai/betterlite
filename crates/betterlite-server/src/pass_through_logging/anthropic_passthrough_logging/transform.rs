use betterlite_anthropic::batches::result::{BatchResponse, project_batch_results};
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{Logging, RawResponse, ResponseRequest, transform_response};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogLevel, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{AnthropicPassthroughError, AnthropicPassthroughInput, TransformTools, logged_json};

// The transform's post_call writes the raw response to the debug log, which never reaches the client.
struct DebugSink;

impl LogSink for DebugSink {
    fn emit(&self, record: LogRecord) {
        match record.level {
            LogLevel::Debug => tracing::debug!("{}", record.message),
            LogLevel::Warning => tracing::warn!("{}", record.message),
            LogLevel::Error => tracing::error!("{}", record.message),
        }
    }
}

// get_anthropic_config: both texts anywhere in the route, in either order, pick the batch results config.
pub(super) fn transformed_response(
    tools: &TransformTools,
    input: &AnthropicPassthroughInput<'_>,
    speed: Option<&str>,
    call_log: &mut LoggingState,
) -> Result<Json, AnthropicPassthroughError> {
    let sink = DebugSink;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &tools.redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: input.end_time.fixed_offset(),
    };
    let call_id = call_log.litellm_call_id.clone();
    let logging = Logging {
        state: call_log,
        env: &env,
    };
    // ModelResponse() starts with a chatcmpl id, which the payload step replaces; a failed payload keeps this call id.
    let mut envelope = ResponseEnvelope::new(format!("chatcmpl-{call_id}"));
    if input.url_route.contains("messages/batches") && input.url_route.contains("results") {
        let response = BatchResponse {
            status: input.status_code,
            headers: input.response_headers,
            body: input.response_text.as_bytes(),
            now: input.end_time.timestamp(),
        };
        project_batch_results(&mut envelope, response, logging, &tools.counter).map_err(
            |source| AnthropicPassthroughError::BatchResults {
                source: Box::new(source),
            },
        )?;
    } else {
        chat_transform(&mut envelope, tools, input, speed, logging)?;
    }
    logged_json(&envelope)
}

// AnthropicConfig.transform_response with no messages, empty request data and the request speed alone.
fn chat_transform(
    envelope: &mut ResponseEnvelope,
    tools: &TransformTools,
    input: &AnthropicPassthroughInput<'_>,
    speed: Option<&str>,
    logging: Logging<'_>,
) -> Result<(), AnthropicPassthroughError> {
    let mut optional_params = IndexMap::new();
    if let Some(requested) = speed {
        optional_params.insert("speed".to_owned(), Json::Text(requested.to_owned()));
    }
    let empty = IndexMap::new();
    let request = ResponseRequest {
        messages: &[],
        optional_params: &optional_params,
        litellm_params: &empty,
        request_data: &empty,
        json_mode: Some(false),
        now: input.end_time.timestamp(),
        disable_add_prefix_to_prompt: false,
    };
    let raw = RawResponse {
        status: input.status_code,
        headers: input.response_headers,
        body: input.response_text.as_bytes(),
    };
    transform_response(envelope, &raw, &request, logging, &tools.counter).map_err(|source| {
        AnthropicPassthroughError::ChatTransform {
            source: Box::new(source),
        }
    })
}
