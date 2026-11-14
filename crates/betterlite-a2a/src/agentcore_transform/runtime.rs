use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use betterlite_core::agents::json::AgentJson;
use betterlite_models::decisions::Json;
use futures_core::Stream;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

use super::SignedRequest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpcMethod {
    Send,
    Stream,
}

impl RpcMethod {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Send => "message/send",
            Self::Stream => "message/stream",
        }
    }
}

// Everything LiteLLM reaches through AmazonAgentCoreConfig, the AWS signer and the A2A httpx client sits behind this seam
pub trait AgentCoreRuntime: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    type Lines: Stream<Item = Result<String, Self::Error>> + Send + 'static;

    fn complete_url(
        &self,
        model: &str,
        optional_params: &BTreeMap<String, AgentJson>,
    ) -> Result<String, Self::Error>;

    fn session_id(
        &self,
        optional_params: &BTreeMap<String, AgentJson>,
        generated: Uuid,
    ) -> Result<String, Self::Error>;

    fn user_id(
        &self,
        optional_params: &BTreeMap<String, AgentJson>,
    ) -> Result<Option<String>, Self::Error>;

    fn sign(
        &self,
        request: &SigningRequest<'_>,
    ) -> impl Future<Output = Result<SignedRequest, Self::Error>> + Send;

    fn post(
        &self,
        request: &HttpRequest<'_>,
    ) -> impl Future<Output = Result<HttpResponse, Self::Error>> + Send;

    fn post_stream(
        &self,
        request: &HttpRequest<'_>,
    ) -> impl Future<Output = Result<StreamedResponse<Self::Lines>, Self::Error>> + Send;
}

pub struct SigningRequest<'call> {
    pub headers: &'call BTreeMap<String, String>,
    pub optional_params: &'call BTreeMap<String, AgentJson>,
    pub request_data: &'call IndexMap<String, Json>,
    pub api_base: &'call str,
    pub model: &'call str,
    pub api_key: Option<&'call SecretString>,
}

pub struct HttpRequest<'call> {
    pub url: &'call str,
    pub headers: &'call BTreeMap<String, String>,
    pub body: &'call [u8],
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub struct StreamedResponse<L> {
    pub status: u16,
    pub content_type: String,
    pub lines: L,
}

pub(crate) const MAX_RESPONSE_BYTES: usize = 0x80_0000;

// httpx hands a JSON reply back as one body; joining its lines with "\n" is the same text for JSON input
pub(crate) async fn collect_body<L, E>(lines: L) -> Result<String, super::AgentCoreError<E>>
where
    L: Stream<Item = Result<String, E>>,
    E: Error + 'static,
{
    let mut lines = std::pin::pin!(lines);
    let mut body = String::new();
    while let Some(line) = lines.next().await {
        let line = line.map_err(super::AgentCoreError::Runtime)?;
        let separator = usize::from(!body.is_empty());
        if body
            .len()
            .saturating_add(separator)
            .saturating_add(line.len())
            > MAX_RESPONSE_BYTES
        {
            return Err(super::AgentCoreError::ResponseTooLarge {
                max: MAX_RESPONSE_BYTES,
            });
        }
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&line);
    }
    Ok(body)
}
