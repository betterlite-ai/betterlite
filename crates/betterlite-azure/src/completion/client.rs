use std::future::Future;
use std::pin::Pin;

use betterlite_models::decisions::Extra;
use betterlite_openai::text_completion_transport::sdk::{RawCompletion, SdkError};
use secrecy::SecretString;

use crate::endpoints::cloudflare::CloudflareParams;
use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::{CallableProvider, ClientKind, LiteLlmParams};

use super::request::Retries;

// completions.with_raw_response.create(**data, timeout=timeout) before .parse()
pub trait CompletionsTransport: Send + Sync {
    fn create<'a>(
        &'a self,
        data: &'a Extra,
        timeout: Option<f64>,
    ) -> Pin<Box<dyn Future<Output = Result<RawCompletion, SdkError>> + Send + 'a>>;
}

// what the handler reads off the client LiteLLM built: isinstance, api_key and _base_url._uri_reference
#[derive(Clone)]
pub struct ClientHandle<Client> {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub base_url: String,
    pub transport: Client,
}

// get_azure_openai_client's arguments at the owned call sites
pub struct GetClient<'a> {
    pub api_key: Option<&'a SecretString>,
    pub api_base: Option<&'a str>,
    pub api_version: Option<&'a str>,
    pub model: Option<&'a str>,
    pub litellm_params: &'a LiteLlmParams,
    pub is_async: bool,
}

// _init_azure_client_for_cloudflare_ai_gateway's arguments
pub struct GatewayClient<'a> {
    pub api_base: &'a str,
    pub model: &'a str,
    pub api_version: Option<&'a str>,
    pub max_retries: Retries,
    pub timeout: Option<f64>,
    pub litellm_params: &'a LiteLlmParams,
    pub cloudflare_params: &'a CloudflareParams,
    pub api_key: Option<&'a SecretString>,
    pub azure_ad_token: Option<&'a SecretString>,
    pub azure_ad_token_provider: Option<&'a CallableProvider>,
    pub acompletion: bool,
}

// B531 derives the client parameters and B173 builds the SDK client; this seam names the two entry points
pub trait ClientSource {
    type Client: CompletionsTransport + Clone;

    fn get(&self, call: GetClient<'_>) -> Result<ClientHandle<Self::Client>, EndpointError>;

    fn gateway(&self, call: GatewayClient<'_>)
    -> Result<ClientHandle<Self::Client>, EndpointError>;
}
