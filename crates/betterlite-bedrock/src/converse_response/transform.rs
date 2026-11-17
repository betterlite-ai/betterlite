use std::convert::Infallible;
use std::error::Error as StdError;
use std::str;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::agentcore_stream::IdSource;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::converse_response::message::{ConvertedMessage, MessageError, convert_message};
use crate::converse_response::request::{RawReply, ReplyRequest};
use crate::converse_response::wire::{Choice, ModelResponse};
use crate::converse_response::{ConverseReply, ReplyFieldError, decode_converse_reply};
use crate::converse_usage::Usage;
use crate::converse_usage::error::{ConverseUsageError, UsageError};
use crate::converse_usage::reasoning::thinking_tokens_from_fields;
use crate::converse_usage::transform::{ReasoningTokenCounter, transform_usage};
use crate::converse_usage::wire::ConverseTokenUsage;
use crate::image_call::plan::LogContext;

#[derive(Debug, thiserror::Error)]
pub enum ClockError {
    #[error("the reply clock is outside the signed 64-bit range")]
    OutOfRange,
}

// The counter failure belongs to the caller's tokenizer, so the usage estimate keeps it as its own type parameter.
#[derive(Debug, thiserror::Error)]
pub enum TransformError<CounterFailure = Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error("bedrock error ({status})")]
    Conversion {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error(transparent)]
    Clock(#[from] ClockError),
    #[error(transparent)]
    Reply(#[from] ReplyFieldError),
    #[error(transparent)]
    Message(#[from] MessageError),
    #[error(transparent)]
    TokenUsage(#[from] ConverseUsageError),
    #[error(transparent)]
    Usage(#[from] UsageError<CounterFailure>),
}

impl<CounterFailure> TransformError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Conversion { status, .. } => Some(*status),
            Self::Header(_)
            | Self::Clock(_)
            | Self::Reply(_)
            | Self::Message(_)
            | Self::TokenUsage(_)
            | Self::Usage(_) => None,
        }
    }
}

pub struct TransformCall<'request, 'log, 'env> {
    pub request: ReplyRequest<'request>,
    pub log: LogContext<'log, 'env>,
    pub now: SystemTime,
}

pub fn transform_response<Counter>(
    call: TransformCall<'_, '_, '_>,
    reply: &RawReply<'_>,
    mut model_response: ModelResponse,
    ids: &dyn IdSource,
    counter: &Counter,
) -> Result<ModelResponse, TransformError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let TransformCall {
        request,
        mut log,
        now,
    } = call;
    log_post_call(&request, reply, &mut log);
    let decoded = match decode_converse_reply(reply.body) {
        Ok(decoded) => decoded,
        Err(error) => {
            return Err(TransformError::Conversion {
                status: 422,
                error: Box::new(error.into_bedrock_error(reply.headers)?),
            });
        }
    };
    let converted = convert_message(&decoded, &request, ids)?;
    let usage = reply_usage(&decoded, &converted, counter)?;
    model_response.choices = vec![Choice {
        finish_reason: converted.finish_reason,
        index: 0,
        message: converted.message,
    }];
    model_response.created = created_seconds(now)?;
    request.model.clone_into(&mut model_response.model);
    model_response.usage = Some(usage);
    apply_optional_fields(&decoded, &mut model_response);
    Ok(model_response)
}

fn log_post_call(request: &ReplyRequest<'_>, reply: &RawReply<'_>, log: &mut LogContext<'_, '_>) {
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Text(response_text(reply.body)),
            input: request.messages.clone(),
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(request.data.clone()),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    );
}

// LiteLLM reads response.text; betterlite names a body that is not UTF-8 by its byte count instead of decoding replacement characters.
fn response_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

fn reply_usage<Counter>(
    reply: &ConverseReply,
    converted: &ConvertedMessage,
    counter: &Counter,
) -> Result<Usage, TransformError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let block = ConverseTokenUsage::decode(reply.usage()?)?;
    let provider_tokens = reply
        .body
        .get("additionalModelResponseFields")
        .and_then(thinking_tokens_from_fields);
    let reasoning = converted.message.reasoning_content.as_deref();
    Ok(transform_usage(
        &block,
        reasoning,
        converted.message.reasoning_content.is_some(),
        provider_tokens,
        counter,
    )?)
}

// int(time.time()): whole seconds, and a clock before the epoch gives the negative it gives Python.
fn created_seconds(now: SystemTime) -> Result<i64, ClockError> {
    let (seconds, negative) = match now.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => (elapsed.as_secs(), false),
        Err(error) => (error.duration().as_secs(), true),
    };
    let whole_seconds = i64::try_from(seconds).map_err(|_error| ClockError::OutOfRange)?;
    if negative {
        return whole_seconds.checked_neg().ok_or(ClockError::OutOfRange);
    }
    Ok(whole_seconds)
}

// Only a guardrail trace and a typed serviceTier reach the envelope; metrics, headers and other top-level members stay with the provider body.
fn apply_optional_fields(reply: &ConverseReply, model_response: &mut ModelResponse) {
    if let Some(trace) = reply.body.get("trace") {
        model_response.trace = Some(trace.clone());
    }
    if let Some(Json::Object(fields)) = reply.body.get("serviceTier")
        && let Some(kind) = fields.get("type")
    {
        model_response.service_tier = Some(kind.clone());
    }
}
