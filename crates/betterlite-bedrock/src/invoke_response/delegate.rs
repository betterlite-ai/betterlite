use std::convert::Infallible;

use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{Logging, RawResponse, ResponseRequest};
use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use http::{HeaderMap, HeaderName, HeaderValue};
use num_traits::cast;

use crate::claude_invoke::response::transform_response as claude_transform_response;
use crate::converse_response::request::{RawReply, ReplyRequest};
use crate::converse_response::transform::TransformCall;
use crate::converse_response::wire::ModelResponse as ConverseResponse;
use crate::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use crate::image_call::plan::LogContext;
use crate::invoke_response::error::{self, InvokeReplyError};
use crate::invoke_response::usage;
use crate::invoke_response::{InvokeReplyCall, RawInvokeReply};
use crate::nova_chat::invoke::transform_response as nova_transform_response;
use crate::pegasus::counter::TokenCounter;
use crate::pegasus::reply::{ReplyContext, decode_pegasus_reply};
use crate::pegasus::response::{PegasusChoice, PegasusMessage, PegasusResponse, PegasusUsage};

pub(super) fn anthropic<Reasoning>(
    call: InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    response: ChatResponse,
    reasoning: &Reasoning,
) -> Result<ResponseEnvelope, InvokeReplyError>
where
    Reasoning: AnthropicReasoningCounter<Failure = Infallible>,
{
    let mut headers = HeaderMap::with_capacity(raw.headers.len());
    for (name, value) in raw.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_error| InvokeReplyError::InvalidHeader { name: name.clone() })?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|_error| InvokeReplyError::InvalidHeader { name: name.clone() })?;
        headers.append(header_name, header_value);
    }
    let request = ResponseRequest {
        messages: message_slice(call.messages),
        optional_params: call.optional_params,
        litellm_params: call.litellm_params,
        request_data: call.request_data,
        json_mode: call.json_mode,
        now: cast::<f64, i64>(call.now).ok_or(InvokeReplyError::ClockOutOfRange)?,
        disable_add_prefix_to_prompt: call.disable_add_prefix_to_prompt,
    };
    let raw_response = RawResponse {
        status: raw.status,
        headers: &headers,
        body: raw.body,
    };
    let mut envelope = ResponseEnvelope::new(response.id);
    envelope.object = response.object;
    envelope.system_fingerprint = response.system_fingerprint;
    let LogContext { state, env } = call.log;
    claude_transform_response(
        &mut envelope,
        &raw_response,
        &request,
        Logging { state, env },
        reasoning,
    )
    .map_err(|source| error::processing_error(raw, &source))?;
    Ok(envelope)
}

pub(super) fn nova<Reasoning>(
    call: InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    response: ChatResponse,
    reasoning: &Reasoning,
) -> Result<ConverseResponse, InvokeReplyError>
where
    Reasoning: ConverseReasoningCounter<Failure = Infallible>,
{
    let seed = ConverseResponse {
        id: response.id,
        created: i64::try_from(response.created)
            .map_err(|_error| InvokeReplyError::CreatedOutOfRange)?,
        model: response.model,
        object: response.object,
        system_fingerprint: response.system_fingerprint,
        choices: Vec::new(),
        usage: None,
        service_tier: response.service_tier.map(Json::Text),
        trace: None,
    };
    let request = ReplyRequest {
        model: call.model,
        messages: call.messages,
        data: RawBody::Object(call.request_data.clone()),
        json_mode: call.json_mode,
        optional: call.optional,
        tool_name_mappings: call.tool_name_mappings,
    };
    let reply = RawReply {
        headers: raw.headers,
        body: raw.body,
    };
    let LogContext { state, env } = call.log;
    nova_transform_response(
        TransformCall {
            request,
            log: LogContext { state, env },
            now: usage::instant(call.now)?,
        },
        &reply,
        seed,
        call.ids,
        reasoning,
    )
    .map_err(|source| error::processing_error(raw, &source))
}

pub(super) async fn pegasus<Counter>(
    call: InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    response: ChatResponse,
    counter: &Counter,
) -> Result<PegasusResponse, InvokeReplyError>
where
    Counter: TokenCounter + Sync,
{
    let created =
        i64::try_from(response.created).map_err(|_error| InvokeReplyError::CreatedOutOfRange)?;
    let choices = response
        .choices
        .into_iter()
        .map(|choice| PegasusChoice {
            index: i64::from(choice.index),
            message: PegasusMessage {
                role: choice.message.role,
                content: choice.message.content,
                tool_calls: choice.message.tool_calls,
            },
            finish_reason: choice.finish_reason,
        })
        .collect();
    let mut seed = PegasusResponse {
        id: response.id,
        choices,
        created,
        model: response.model,
        usage: PegasusUsage::default(),
    };
    let context = ReplyContext {
        status: raw.status,
        headers: raw.headers,
        model: call.model,
        messages: call.counted_messages,
        now: call.now,
    };
    decode_pegasus_reply(raw.body, &context, &mut seed, counter)
        .await
        .map_err(|source| error::processing_error(raw, &source))?;
    Ok(seed)
}

fn message_slice(messages: &Json) -> &[Json] {
    match messages {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => &[],
    }
}
