use std::convert::Infallible;
use std::str;
use std::time::{Duration, SystemTime};

use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
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
    RerankHttpReply, RerankHttpRequest, RerankTransport as _, TransferError,
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
    pub counter: &'log dyn ReasoningTokenCounter<Failure = Infallible>,
    pub log: LogContext<'log, 'env>,
}

pub struct BufferedReply {
    pub response: ModelResponse,
    pub provider_headers: ProviderHeaderProjection,
}

// Conversion and logging happen after the Send transport stage has completed.
pub(super) fn finish_buffered(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    reply: &RerankHttpReply,
    tail: BufferedTail<'_, '_>,
) -> Result<BufferedReply, ConverseCallError> {
    let raw_headers: IndexMap<String, String> = reply.headers.iter().cloned().collect();
    let provider_headers =
        set_provider_response_headers(&AdditionalHeaders::default(), &raw_headers);
    let raw = RawReply {
        headers: &reply.headers,
        body: &reply.body,
    };
    let response = transform_response(
        TransformCall {
            request: ReplyRequest {
                model: call.model,
                messages: &plan.messages,
                data: RawBody::Text(plan.data.clone()),
                json_mode: None,
                optional: &plan.optional,
                tool_name_mappings: &plan.tool_name_mappings,
            },
            log: tail.log,
            now: tail.now,
        },
        &raw,
        call.model_response.clone(),
        tail.ids,
        &ReasoningCounter(tail.counter),
    )?;
    Ok(BufferedReply {
        response,
        provider_headers,
    })
}

// A matching supplied client keeps its own timeout; the factory's client gets the call's, and the async route asks it for the logging transport while the sync route posts without one.
pub(super) async fn send_buffered<Factory>(
    factory: &Factory,
    parts: &BufferedParts<'_, '_, Factory::Buffered>,
    plan: &CallPlan,
    state: &LoggingState,
) -> Result<RerankHttpReply, ConverseCallError>
where
    Factory: ConverseClientFactory,
{
    let built;
    let transport: &Factory::Buffered = match (parts.route, parts.supplied) {
        (ClientKind::Asynchronous, Some(client)) if client.kind == ClientKind::Asynchronous => {
            client.transport
        }
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
    let outcome = match parts.route {
        ClientKind::Asynchronous => transport.post_with_logging(http, state).await,
        ClientKind::Sync => transport.post(http).await,
    };
    let reply = match outcome {
        Ok(response) => response,
        Err(TransferError::Timeout) => return Err(timeout_error()?),
        Err(error) => return Err(ConverseCallError::Transport(error)),
    };
    if !(200..=299).contains(&reply.status) {
        return Err(provider_error(&plan.url, &reply)?);
    }
    Ok(reply)
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

struct ReasoningCounter<'counter>(&'counter dyn ReasoningTokenCounter<Failure = Infallible>);

impl ReasoningTokenCounter for ReasoningCounter<'_> {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, text: &str) -> Result<i64, Self::Failure> {
        self.0.count_reasoning_tokens(text)
    }
}

#[path = "tests/buffered.rs"]
mod tests;
