use betterlite_models::decisions::Extra;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use uuid::Uuid;

use crate::endpoints::cloudflare::CloudflareParams;
use crate::endpoints::projection::{CallableProvider, LiteLlmParams};

use super::client::ClientHandle;
use super::request::Retries;

// AzureTextCompletion.completion's arguments, with the caller's optional_params dict it pops from
pub struct CompletionCall<'a, Client> {
    pub model: Option<&'a str>,
    pub messages: Option<&'a [Message]>,
    pub api_key: Option<&'a SecretString>,
    pub api_base: Option<&'a str>,
    pub api_version: Option<&'a str>,
    pub azure_ad_token: Option<&'a SecretString>,
    pub azure_ad_token_provider: Option<&'a CallableProvider>,
    pub timeout: Option<f64>,
    pub optional_params: &'a mut Extra,
    pub litellm_params: LiteLlmParams,
    pub cloudflare_params: CloudflareParams,
    pub acompletion: bool,
    pub headers: Option<&'a SecretHeaders>,
    pub supplied: Option<ClientHandle<Client>>,
}

// The request the dispatch built plus the client parameters each inner call carries
pub struct SentCall<'a, Client> {
    pub model: String,
    pub data: Extra,
    pub prompt: String,
    pub max_retries: Retries,
    pub litellm_params: &'a LiteLlmParams,
    pub supplied: Option<ClientHandle<Client>>,
}

// The clocks and id sources the owned functions read instead of the process clock
pub struct Runtime<'a> {
    pub prompts: &'a PromptFactory,
    pub now: DateTime<Utc>,
    pub identifier: &'a dyn Fn() -> Uuid,
    pub clock: &'a dyn Fn() -> i64,
    pub stream_now: fn() -> StreamInstant,
    pub stream_id: fn() -> Uuid,
}
