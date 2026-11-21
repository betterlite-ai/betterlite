use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_config::sections::openai_chat_content::Settings as ChatContentSettings;
use betterlite_models::decisions;
use betterlite_openai::output_limits::tokens::PromptTokens;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::credentials::source::CredentialSources;
use crate::endpoints::cloudflare::CloudflareParams;
use crate::endpoints::projection::{CallableProvider, LiteLlmParams};

use super::client::ClientHandle;
use super::request::Retries;

// AzureChatCompletion.completion's arguments, with the caller's optional_params dict it pops from
pub struct ChatCall<'call, Client> {
    pub model: Option<&'call str>,
    pub messages: Option<&'call [Message]>,
    pub api_key: Option<&'call SecretString>,
    pub api_base: &'call str,
    pub api_version: &'call str,
    pub azure_ad_token: Option<&'call SecretString>,
    pub azure_ad_token_provider: Option<&'call CallableProvider>,
    // streaming() and async_streaming() take it and never read it
    pub dynamic_params: bool,
    pub timeout: f64,
    pub optional_params: &'call mut Object,
    pub litellm_params: LiteLlmParams,
    // litellm_params.get("base_model"), which picks the GPT-5 request transform
    pub base_model: Option<&'call str>,
    pub cloudflare_params: CloudflareParams,
    pub acompletion: bool,
    pub headers: Option<&'call SecretHeaders>,
    pub supplied: Option<ClientHandle<Client>>,
    // the credential lookup B685's streaming adapters need for an oidc/ token
    pub sources: &'call (dyn CredentialSources + Sync),
    // B531's DEFAULT_MAX_RETRIES loader and litellm_settings.openai_system_messages_first
    pub azure: &'call AzureSettings,
    pub chat: &'call ChatContentSettings,
    pub cancel: Option<&'call CancellationToken>,
}

// The request the dispatch built plus the client parameters each inner call carries
pub struct SentChat<Client> {
    pub model: String,
    // the messages the sync pre and post calls log, before any transform
    pub messages: decisions::Json,
    pub data: Object,
    pub max_retries: Retries,
    pub json_mode: bool,
    pub supplied: Option<ClientHandle<Client>>,
    pub litellm_params: LiteLlmParams,
}

// The clocks, id source and token counter the owned functions read instead of the process ones
pub struct Runtime<'runtime> {
    pub identifier: fn() -> Uuid,
    pub clock: fn() -> i64,
    pub recovered_clock: fn() -> u64,
    pub now: fn() -> Duration,
    pub stream_now: fn() -> StreamInstant,
    pub prompt_tokens: &'runtime dyn PromptTokens,
}
