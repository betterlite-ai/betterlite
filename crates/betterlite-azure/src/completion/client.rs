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
    fn create<'call>(
        &'call self,
        data: &'call Extra,
        timeout: Option<f64>,
    ) -> Pin<Box<dyn Future<Output = Result<RawCompletion, SdkError>> + Send + 'call>>;
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
pub struct GetClient<'get> {
    pub api_key: Option<&'get SecretString>,
    pub api_base: Option<&'get str>,
    pub api_version: Option<&'get str>,
    pub model: Option<&'get str>,
    pub litellm_params: &'get LiteLlmParams,
    pub is_async: bool,
}

// _init_azure_client_for_cloudflare_ai_gateway's arguments
pub struct GatewayClient<'gateway> {
    pub api_base: &'gateway str,
    pub model: &'gateway str,
    pub api_version: Option<&'gateway str>,
    pub max_retries: Retries,
    pub timeout: Option<f64>,
    pub litellm_params: &'gateway LiteLlmParams,
    pub cloudflare_params: &'gateway CloudflareParams,
    pub api_key: Option<&'gateway SecretString>,
    pub azure_ad_token: Option<&'gateway SecretString>,
    pub azure_ad_token_provider: Option<&'gateway CallableProvider>,
    pub acompletion: bool,
}

// B531 derives the client parameters and B173 builds the SDK client; this seam names the two entry points
pub trait ClientSource: Send + Sync {
    type Client: CompletionsTransport + Clone;

    fn get(&self, call: GetClient<'_>) -> Result<ClientHandle<Self::Client>, EndpointError>;

    fn gateway(&self, call: GatewayClient<'_>)
    -> Result<ClientHandle<Self::Client>, EndpointError>;
}
