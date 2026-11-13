use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use betterlite_core::agents::json::AgentJson;
use betterlite_models::decisions::Json;
use futures_core::Stream;
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

// Everything LiteLLM reaches through AmazonAgentCoreConfig, the AWS signer and the A2A httpx client,
// behind one injected seam: the URL from the model ARN, the runtime identifiers, the signature and the wire.
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
