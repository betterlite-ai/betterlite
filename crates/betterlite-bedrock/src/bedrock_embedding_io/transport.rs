use std::fmt::{self, Formatter};
use std::future::Future;
use std::str;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

use super::decode::decode_embedding_response;
use super::error::EmbeddingIoError;
use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};

// get_async_httpx_client is built with litellm.LlmProviders.BEDROCK
pub const ASYNC_CLIENT_PROVIDER: &str = "bedrock";

const TIMEOUT_STATUS: u16 = 408;
const TIMEOUT_MESSAGE: &str = "Timeout error occurred.";

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

// the signed headers carry the provider credential, so Debug names them without their values
impl fmt::Debug for EmbeddingRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("EmbeddingRequest")
            .field("url", &self.url)
            .field("header_names", &names)
            .field("body_length", &self.body.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyRequest {
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    // the request that produced this reply, after the transport's redirects
    pub request: ReplyRequest,
}

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("the provider request timed out")]
    Timeout,
    #[error(transparent)]
    Failed(#[from] TransportError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Timeout {
    Seconds(f64),
    Record(Box<TimeoutRecord>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeoutRecord {
    pub connect: f64,
    pub read: f64,
    pub write: f64,
    pub pool: f64,
}

impl Timeout {
    // httpx.Timeout(2.5) sets connect, read, write and pool to 2.5
    #[must_use]
    pub const fn fields(&self) -> [f64; 4] {
        match self {
            Self::Seconds(seconds) => [*seconds, *seconds, *seconds, *seconds],
            Self::Record(record) => [record.connect, record.read, record.write, record.pool],
        }
    }
}

pub trait EmbeddingTransport: Send + Sync {
    fn post(
        &self,
        request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send;
}

// the supplied client's kind decides whether LiteLLM accepts it on the sync or the async path
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Async,
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

// B173 owns client construction; this seam receives the client with the timeout LiteLLM passed it
pub trait ClientFactory {
    type Client: EmbeddingTransport;

    fn sync_client(&self, timeout: Option<Timeout>) -> Self::Client;

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client;
}

pub struct EmbeddingCall<'source> {
    pub asynchronous: bool,
    pub url: &'source str,
    pub headers: &'source [(String, String)],
    pub data: &'source Object,
    pub timeout: Option<Timeout>,
}

pub async fn post_embedding<Factory>(
    call: &EmbeddingCall<'_>,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    factory: &Factory,
) -> Result<Object, EmbeddingIoError>
where
    Factory: ClientFactory + Sync,
{
    let request = EmbeddingRequest {
        url: call.url.to_owned(),
        headers: call.headers.to_vec(),
        body: serde_json::to_vec(call.data)
            .map_err(|source| EmbeddingIoError::Encode { source })?,
    };
    let reply = match send(call, request, supplied, factory).await {
        Ok(reply) => reply,
        Err(TransferError::Timeout) => return Err(timeout_error()?),
        Err(error @ TransferError::Failed(_)) => return Err(EmbeddingIoError::Transport(error)),
    };
    // httpx raise_for_status raises for every status outside 2xx
    if !(200..300).contains(&reply.status) {
        return Err(provider_error(&reply)?);
    }
    decode_embedding_response(&reply.body)
}

// a supplied client of the other handler kind does not count, so a fresh client is built with the timeout
async fn send<Factory>(
    call: &EmbeddingCall<'_>,
    request: EmbeddingRequest,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    factory: &Factory,
) -> Result<EmbeddingReply, TransferError>
where
    Factory: ClientFactory + Sync,
{
    match (call.asynchronous, supplied) {
        (true, Some(client)) if client.kind == ClientKind::Async => {
            client.transport.post(request).await
        }
        (true, _) => {
            factory
                .async_client(ASYNC_CLIENT_PROVIDER, call.timeout.clone())
                .post(request)
                .await
        }
        (false, Some(client)) if client.kind == ClientKind::Sync => {
            client.transport.post(request).await
        }
        (false, _) => {
            factory
                .sync_client(call.timeout.clone())
                .post(request)
                .await
        }
    }
}

fn timeout_error() -> Result<EmbeddingIoError, HeaderError> {
    let error = BedrockError::try_from(ErrorInput {
        status_code: TIMEOUT_STATUS,
        message: TIMEOUT_MESSAGE.to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })?;
    Ok(EmbeddingIoError::Provider {
        status: TIMEOUT_STATUS,
        error: Box::new(error),
    })
}

fn provider_error(reply: &EmbeddingReply) -> Result<EmbeddingIoError, HeaderError> {
    let message = response_text(&reply.body);
    let request = ErrorRequest {
        method: reply.request.method.clone(),
        url: reply.request.url.clone(),
    };
    let response = ErrorResponse {
        status_code: reply.status,
        request,
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
    Ok(EmbeddingIoError::Provider {
        status: reply.status,
        error: Box::new(error),
    })
}

// LiteLLM reads response.text; bytes that are not UTF-8 are refused by naming their count
fn response_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}
