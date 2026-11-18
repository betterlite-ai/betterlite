use std::convert::Infallible;
use std::str;
use std::time::{Duration, SystemTime};

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use indexmap::IndexMap;

use crate::agentcore_stream::IdSource;
use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};
use crate::converse_response::request::{RawReply, ReplyRequest};
use crate::converse_response::transform::{TransformCall, transform_response};
use crate::converse_response::wire::ModelResponse;
use crate::converse_usage::transform::ReasoningTokenCounter;
use crate::image_call::plan::LogContext;
use crate::rerank::transport::{
    ClientKind, RerankHttpReply, RerankHttpRequest, RerankTransport as _, SuppliedClient,
    TransferError,
};

use super::ConverseCall;
use super::error::ConverseCallError;
use super::prepare::CallPlan;
use super::transport::ConverseClientFactory;

pub struct BufferedParts<'parts, 'deps, Client> {
    pub route: ClientKind,
    pub timeout: Option<Duration>,
    pub supplied: Option<&'parts SuppliedClient<'deps, Client>>,
}

pub struct BufferedTail<'log, 'env> {
    pub now: SystemTime,
    pub ids: &'log dyn IdSource,
    pub log: LogContext<'log, 'env>,
}

pub struct BufferedReply {
    pub response: ModelResponse,
    pub provider_headers: ProviderHeaderProjection,
}

// The POST, the raise_for_status gate and the buffered conversion LiteLLM runs before returning.
pub(crate) async fn send_buffered<Factory>(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    parts: &BufferedParts<'_, '_, Factory::Buffered>,
    factory: &Factory,
    mut tail: BufferedTail<'_, '_>,
) -> Result<BufferedReply, ConverseCallError>
where
    Factory: ConverseClientFactory,
{
    let reply = post(factory, parts, call, plan, &*tail.log.state).await?;
    if !(200..=299).contains(&reply.status) {
        return Err(provider_error(&plan.url, &reply)?);
    }
    let provider_headers: IndexMap<String, String> = reply.headers.iter().cloned().collect();
    let provider_headers =
        set_provider_response_headers(&AdditionalHeaders::default(), &provider_headers);
    let response = transform_reply(call, plan, &reply, &mut tail)?;
    Ok(BufferedReply {
        response,
        provider_headers,
    })
}

// A matching supplied client keeps its own timeout; the factory's client gets the call's, and the
// async route asks it for the logging transport while the sync route posts without one.
async fn post<Factory>(
    factory: &Factory,
    parts: &BufferedParts<'_, '_, Factory::Buffered>,
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    state: &LoggingState,
) -> Result<RerankHttpReply, ConverseCallError>
where
    Factory: ConverseClientFactory,
{
    let built;
    let transport: &Factory::Buffered = match (parts.route, parts.supplied) {
        (ClientKind::Async, Some(client)) if client.kind == ClientKind::Async => client.transport,
        (ClientKind::Sync, Some(client)) if client.kind == ClientKind::Sync => client.transport,
        _ => {
            built = factory.buffered(parts.route, parts.timeout);
            &built
        }
    };
    let http = RerankHttpRequest {
        url: plan.url.clone(),
        headers: plan.signed.headers.clone(),
        body: plan.signed.body.clone(),
        timeout: None,
    };
    let reply = match parts.route {
        ClientKind::Async => transport.post_with_logging(http, state).await,
        ClientKind::Sync => transport.post(http).await,
    };
    match reply {
        Ok(reply) => Ok(reply),
        Err(TransferError::Timeout) => Err(timeout_error()?),
        Err(error) => Err(ConverseCallError::Transport(error)),
    }
}

// The reply conversion reads api_key as the empty string and takes stream only from the bool the
// caller passed, which the handler already turned into false unless it was a literal bool.
fn transform_reply(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    reply: &RerankHttpReply,
    tail: &mut BufferedTail<'_, '_>,
) -> Result<ModelResponse, ConverseCallError> {
    let log = LogContext {
        state: &mut *tail.log.state,
        env: tail.log.env,
    };
    let raw = RawReply {
        headers: &reply.headers,
        body: &reply.body,
    };
    Ok(transform_response(
        TransformCall {
            request: ReplyRequest {
                model: call.model,
                messages: &plan.messages,
                data: RawBody::Text(plan.data.clone()),
                json_mode: None,
                optional: &plan.optional,
                tool_name_mappings: &plan.tool_name_mappings,
            },
            log,
            now: tail.now,
        },
        &raw,
        call.model_response.clone(),
        tail.ids,
        &NoReasoningTokens,
    )?)
}

fn timeout_error() -> Result<ConverseCallError, HeaderError> {
    let error = BedrockError::try_from(ErrorInput {
        status_code: 408,
        message: "Timeout error occurred.".to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })?;
    Ok(ConverseCallError::Provider {
        status: 408,
        error: Box::new(error),
    })
}

fn provider_error(url: &str, reply: &RerankHttpReply) -> Result<ConverseCallError, HeaderError> {
    let message = response_text(&reply.body);
    let response = ErrorResponse {
        status_code: reply.status,
        request: ErrorRequest {
            method: "POST".to_owned(),
            url: url.to_owned(),
        },
    };
    let error = BedrockError::try_from(ErrorInput {
        status_code: reply.status,
        message: message.clone(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(reply.headers.clone())),
        response: Some(ErrorReply {
            response,
            headers: reply.headers.clone(),
            body: ResponseBody::Read(message),
        }),
    })?;
    Ok(ConverseCallError::Provider {
        status: reply.status,
        error: Box::new(error),
    })
}

// LiteLLM reads response.text; betterlite names a body that is not UTF-8 by its byte count.
fn response_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_error| format!("{} bytes that are not text", body.len()),
        str::to_owned,
    )
}

// The buffered conversion owns no tokenizer, so it answers every reasoning estimate with zero.
struct NoReasoningTokens;

impl ReasoningTokenCounter for NoReasoningTokens {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        Ok(0)
    }
}
