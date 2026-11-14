use std::collections::BTreeMap;
use std::error::Error;

use betterlite_core::agents::json::AgentJson;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::wire::MessageSendParams;
use crate::utils::session_scoping::{get_session_id_from_a2a_params, scope_session_to_principal};
use call::{AgentCoreHandler, HandlerCall, SignedRequestCall, StreamedEvents};
use headers::filter_reserved_headers;
use runtime::{
    AgentCoreRuntime, HttpRequest, HttpResponse, RpcMethod, SigningRequest, StreamedResponse,
};
use sse::{decode_body, replies};

pub mod call;
pub mod config;
mod headers;
pub mod params;
pub mod runtime;
pub mod sse;
mod tests;

// LiteLLM's _filter_reserved_headers, _request_scoped_runtime_session_id and _validate_runtime_session_id (which raises BadRequestError) are the three helpers below.
const RUNTIME_SESSION_ID_MIN_LENGTH: usize = 33;
const RUNTIME_SESSION_ID_MAX_LENGTH: usize = 256;
const SESSION_HEADER: &str = "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id";
const USER_HEADER: &str = "X-Amzn-Bedrock-AgentCore-Runtime-User-Id";
const RESERVED_EXACT_HEADERS: [&str; 2] = ["authorization", "host"];
const RESERVED_PREFIX_HEADERS: [&str; 2] = ["x-amzn-bedrock-agentcore-runtime-", "x-amz-"];

#[derive(Debug, Error)]
pub enum TransformError {
    #[error(
        "litellm_params is required for BedrockAgentCoreA2AConfig (must contain model with AgentCore ARN)"
    )]
    MissingParams,
    #[error("the AgentCore runtime session id must be 33 to 256 characters, and this one is {} characters", .session_id.chars().count())]
    SessionId { session_id: String },
    #[error("the AgentCore request field '{field}' must be a string")]
    FieldType { field: &'static str },
    #[error("the AgentCore message parameters could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the AgentCore response is not JSON")]
    Decode(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum AgentCoreError<E: Error + 'static> {
    #[error(transparent)]
    Transform(#[from] TransformError),
    #[error("the AgentCore runtime answered with HTTP status {status}")]
    Status { status: u16 },
    #[error("the AgentCore runtime reply is larger than {max} bytes")]
    ResponseTooLarge { max: usize },
    #[error(transparent)]
    Runtime(E),
}

pub struct SignedRequest {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

fn scoped_session_id(params: &MessageSendParams, principal: Option<&str>) -> Option<String> {
    let context_id = get_session_id_from_a2a_params(params).ok().flatten()?;
    if context_id.is_empty() {
        return None;
    }
    Some(scope_session_to_principal(&context_id, principal))
}

fn validate_runtime_session_id(session_id: &str) -> Result<String, TransformError> {
    let length = session_id.chars().count();
    if !(RUNTIME_SESSION_ID_MIN_LENGTH..=RUNTIME_SESSION_ID_MAX_LENGTH).contains(&length) {
        return Err(TransformError::SessionId {
            session_id: session_id.to_owned(),
        });
    }
    Ok(session_id.to_owned())
}

fn envelope(
    request_id: &str,
    method: RpcMethod,
    params: &MessageSendParams,
) -> Result<IndexMap<String, Json>, TransformError> {
    let encoded = serde_json::to_value(params).map_err(TransformError::Encode)?;
    let encoded_params = serde_json::from_value(encoded).map_err(TransformError::Encode)?;
    Ok(IndexMap::from([
        ("jsonrpc".to_owned(), Json::Text("2.0".to_owned())),
        ("method".to_owned(), Json::Text(method.name().to_owned())),
        ("id".to_owned(), Json::Text(request_id.to_owned())),
        ("params".to_owned(), encoded_params),
    ]))
}

fn request_headers<R, S>(
    runtime: &R,
    call: &SignedRequestCall<'_>,
    ids: &mut S,
) -> Result<BTreeMap<String, String>, AgentCoreError<R::Error>>
where
    R: AgentCoreRuntime,
    S: EventSource + Send,
{
    let optional_params = &call.litellm_params.optional;
    let mut headers = BTreeMap::new();
    let derived = scoped_session_id(call.params, call.litellm_params.principal.as_deref());
    let session_id = match derived {
        Some(scoped) => scoped,
        None => runtime
            .session_id(optional_params, ids.next_uuid())
            .map_err(AgentCoreError::Runtime)?,
    };
    headers.insert(
        SESSION_HEADER.to_owned(),
        validate_runtime_session_id(&session_id)?,
    );
    let configured = runtime
        .user_id(optional_params)
        .map_err(AgentCoreError::Runtime)?;
    if let Some(user_id) = configured.filter(|value| !value.is_empty()) {
        headers.insert(USER_HEADER.to_owned(), user_id);
    }
    if let Some(extra_headers) = call.agent_extra_headers {
        headers.extend(filter_reserved_headers(extra_headers));
    }
    Ok(headers)
}

// LiteLLM's fn get_url_and_signed_request returns (url, signed_headers, signed_body_bytes), here a SignedRequest -> never a missing body
pub async fn get_url_and_signed_request<R, S>(
    runtime: &R,
    call: &SignedRequestCall<'_>,
    ids: &mut S,
) -> Result<SignedRequest, AgentCoreError<R::Error>>
where
    R: AgentCoreRuntime,
    S: EventSource + Send,
{
    let model = call.litellm_params.model.as_str();
    let agentcore_model = model.strip_prefix("bedrock/").unwrap_or(model);
    let optional_params = &call.litellm_params.optional;
    let url = runtime
        .complete_url(agentcore_model, optional_params)
        .map_err(AgentCoreError::Runtime)?;
    let request_data = envelope(call.request_id, call.method, call.params)?;
    let headers = request_headers(runtime, call, ids)?;
    let request = SigningRequest {
        headers: &headers,
        optional_params,
        request_data: &request_data,
        api_base: &url,
        model: agentcore_model,
        api_key: call.litellm_params.api_key.as_ref(),
    };
    runtime
        .sign(&request)
        .await
        .map_err(AgentCoreError::Runtime)
}

fn check_status<E>(status: u16) -> Result<(), AgentCoreError<E>>
where
    E: Error + 'static,
{
    if (200..300).contains(&status) {
        return Ok(());
    }
    Err(AgentCoreError::Status { status })
}

impl SignedRequest {
    const fn request(&self) -> HttpRequest<'_> {
        HttpRequest {
            url: &self.url,
            headers: &self.headers,
            body: &self.body,
        }
    }
}

type Outcome<T, R> = Result<T, AgentCoreError<<R as AgentCoreRuntime>::Error>>;
type Events<R> = StreamedEvents<<R as AgentCoreRuntime>::Error>;
type Streamed<R> = StreamedResponse<<R as AgentCoreRuntime>::Lines>;

impl<R, S> AgentCoreHandler<R, S>
where
    R: AgentCoreRuntime,
    S: EventSource + Send,
{
    async fn signed(
        &mut self,
        call: &HandlerCall<'_>,
        method: RpcMethod,
    ) -> Outcome<SignedRequest, R> {
        let signed = get_url_and_signed_request(
            &self.runtime,
            &SignedRequestCall {
                request_id: call.request_id,
                params: call.params,
                litellm_params: &call.litellm_params,
                method,
                agent_extra_headers: call.agent_extra_headers,
            },
            &mut self.ids,
        )
        .await?;
        tracing::info!(url = %signed.url, method = method.name(), "BedrockAgentCore A2A: sending a request");
        Ok(signed)
    }

    // LiteLLM's client.post(url, headers=headers, data=body), with the httpx client behind the runtime seam
    async fn sent(runtime: &R, signed: &SignedRequest) -> Outcome<HttpResponse, R> {
        let request = signed.request();
        let posted = runtime.post(&request).await;
        let response = posted.map_err(AgentCoreError::Runtime)?;
        check_status::<R::Error>(response.status)?;
        Ok(response)
    }

    async fn opened(runtime: &R, signed: &SignedRequest) -> Outcome<Streamed<R>, R> {
        let request = signed.request();
        let posted = runtime.post_stream(&request).await;
        let response = posted.map_err(AgentCoreError::Runtime)?;
        check_status::<R::Error>(response.status)?;
        Ok(response)
    }

    pub async fn handle_non_streaming(&mut self, call: &HandlerCall<'_>) -> Outcome<AgentJson, R> {
        let signed = self.signed(call, RpcMethod::Send).await?;
        let response = Self::sent(&self.runtime, &signed).await?;
        Ok(decode_body(&response.body)?)
    }

    pub async fn handle_streaming(&mut self, call: &HandlerCall<'_>) -> Outcome<Events<R>, R> {
        let signed = self.signed(call, RpcMethod::Stream).await?;
        let response = Self::opened(&self.runtime, &signed).await?;
        Ok(replies(response))
    }
}
