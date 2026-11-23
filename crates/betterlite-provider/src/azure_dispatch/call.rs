use betterlite_azure::azure_chat_base_policy::AzureChatSettings;
use betterlite_config::loader::EnvironmentLookup as _;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{Headers, OpenAiSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default)]
pub struct Globals {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub azure_default_api_version: Option<String>,
}
#[derive(Clone, Default)]
pub struct Composition {
    pub environment: BTreeMap<String, String>,
    pub globals: Globals,
    pub chat_defaults: AzureChatSettings,
    pub o_series_defaults: OpenAiSettings,
    pub openai_system_messages_first: bool,
}
impl Composition {
    #[must_use]
    pub fn from_environment(environment: BTreeMap<String, String>) -> Self {
        Self {
            environment,
            globals: Globals::default(),
            chat_defaults: AzureChatSettings::default(),
            o_series_defaults: OpenAiSettings::default(),
            openai_system_messages_first: false,
        }
    }
    // an environment secret is unset when the variable is missing or empty
    #[must_use]
    pub fn secret(&self, name: &str) -> Option<SecretString> {
        self.environment
            .lookup(name)
            .filter(|text| !text.is_empty())
            .map(SecretString::from)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Chat,
    Text,
    Embedding,
}
// What the ranks resolve to for one call, so a second call cannot reuse the first call's identity.
pub struct Resolved {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_type: String,
    // the token the SDK authenticates with: only the call's own azure_ad_token parameter reaches it
    pub auth_token: Option<SecretString>,
    // main.py's resolved token: extra_body.azure_ad_token, then AZURE_AD_TOKEN
    pub azure_ad_token: Option<SecretString>,
    // the call's own headers and extra headers, and litellm.headers, each kept for its own merge
    pub headers: Option<Headers>,
    pub extra_headers: Option<Headers>,
    pub globals_headers: Option<Headers>,
    pub max_retries: Option<Json>,
    pub extra_body: Object,
    pub default_api_version: String,
    pub o_series: bool,
}
#[derive(Clone, Default)]
pub struct Credentials {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub extra_headers: Option<Headers>,
    // kwargs.max_retries: chat inserts it when it is not None, text never inserts it
    pub max_retries: Option<Json>,
    pub extra_body: Option<Json>,
    pub azure_ad_token: Option<SecretString>,
    pub base_model: Option<String>,
    pub timeout: Option<f64>,
}
#[derive(Debug, Clone)]
pub struct SuppliedClient {
    pub api_version: Option<String>,
}
#[derive(Clone)]
pub struct ChatCall {
    pub model: String,
    pub client_model: Option<String>,
    pub messages: Vec<Message>,
    pub optional: Object,
    pub credentials: Credentials,
    pub stream: bool,
    pub supplied: Option<SuppliedClient>,
}
#[derive(Clone)]
pub struct TextCall {
    pub model: String,
    pub messages: Vec<Message>,
    pub optional: Object,
    pub credentials: Credentials,
    pub acompletion: bool,
}
#[derive(Clone)]
pub struct EmbeddingCall {
    pub model: String,
    pub request: EmbeddingRequest,
    pub credentials: Credentials,
}
pub enum Request {
    Chat(Box<ChatCall>),
    Text(Box<TextCall>),
    Embedding(Box<EmbeddingCall>),
}
