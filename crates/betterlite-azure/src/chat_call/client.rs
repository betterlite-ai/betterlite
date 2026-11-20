use betterlite_openai::output_limits::raw_send::{AsyncRawChatTransport, SyncRawChatTransport};
use secrecy::SecretString;

use crate::azure_stream_errors::records::{GetStreamClient, StreamClient};
use crate::endpoints::cloudflare::CloudflareParams;
use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::{CallableProvider, ClientKind, LiteLlmParams};

use super::request::Retries;

// what the handlers read off the client LiteLLM built: isinstance, api_key and _base_url._uri_reference
#[derive(Clone)]
pub struct ClientHandle<Client> {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub base_url: String,
    pub transport: Client,
}

// get_azure_openai_client's arguments at the owned call sites
pub struct GetClient<'get, Client> {
    pub api_key: Option<&'get SecretString>,
    pub api_base: Option<&'get str>,
    pub api_version: Option<&'get str>,
    pub model: Option<&'get str>,
    pub litellm_params: &'get LiteLlmParams,
    pub is_async: bool,
    // get_azure_openai_client hands a supplied client back as it is
    pub supplied: Option<&'get ClientHandle<Client>>,
}

// _init_azure_client_for_cloudflare_ai_gateway's arguments
pub struct GatewayClient<'gateway> {
    pub api_base: &'gateway str,
    pub model: &'gateway str,
    pub api_version: Option<&'gateway str>,
    pub max_retries: Retries,
    pub timeout: f64,
    pub litellm_params: &'gateway LiteLlmParams,
    pub cloudflare_params: &'gateway CloudflareParams,
    pub api_key: Option<&'gateway SecretString>,
    pub azure_ad_token: Option<&'gateway SecretString>,
    pub azure_ad_token_provider: Option<&'gateway CallableProvider>,
    pub acompletion: bool,
}

// B531 derives the client parameters and B173 builds the SDK client; this seam names the entry points
pub trait ClientSource: Send + Sync {
    // the transport is the openai SDK's chat.completions.with_raw_response.create
    type Client: SyncRawChatTransport + AsyncRawChatTransport + Clone + Send + Sync;

    fn get(
        &self,
        call: GetClient<'_, Self::Client>,
    ) -> Result<ClientHandle<Self::Client>, EndpointError>;

    fn gateway(&self, call: GatewayClient<'_>)
    -> Result<ClientHandle<Self::Client>, EndpointError>;

    // the streaming branch hands the client completion already holds to B685's adapter
    fn stream(
        &self,
        supplied: Option<&ClientHandle<Self::Client>>,
        call: GetStreamClient<'_>,
    ) -> Result<StreamClient, EndpointError>;
}

// Sync accepts the Azure and OpenAI spellings, and OnlyOther names the kind neither guard lets through
#[must_use]
pub const fn is_sync_chat(kind: ClientKind) -> bool {
    matches!(kind, ClientKind::AzureOpenAi | ClientKind::OpenAi)
}

#[must_use]
pub const fn is_async_chat(kind: ClientKind) -> bool {
    matches!(kind, ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi)
}
