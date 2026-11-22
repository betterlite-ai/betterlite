use betterlite_azure::azure_chat_base_policy::AzureChatSettings;
use betterlite_config::loader::EnvironmentLookup as _;
use betterlite_config::loader::startup::process_environment;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{Headers, OpenAiSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use std::collections::BTreeMap;
#[derive(Debug, Default)]
pub struct Globals {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub azure_default_api_version: Option<String>,
}
#[derive(Default)]
pub struct Composition {
    pub environment: BTreeMap<String, String>,
    pub globals: Globals,
    pub chat_defaults: AzureChatSettings,
    pub o_series_defaults: OpenAiSettings,
    pub openai_system_messages_first: bool,
}
const AZURE_ENVIRONMENT: &str = "AZURE_API_BASE AZURE_API_VERSION AZURE_API_TYPE AZURE_OPENAI_API_KEY AZURE_API_KEY AZURE_AD_TOKEN AZURE_DEFAULT_API_VERSION";
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
    // the process environment is not a test's to set, so no test can change what this reads
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn process() -> Self {
        let azure = process_environment()
            .into_iter()
            .filter(|(name, _value)| AZURE_ENVIRONMENT.split(' ').any(|key| key == name.as_str()))
            .collect();
        Self::from_environment(azure)
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
impl SuppliedClient {
    // _check_dynamic_azure_params: only a non-None version that differs from the client's discards it
    #[must_use]
    pub fn discarded(&self, api_version: Option<&str>) -> bool {
        match api_version {
            None => false,
            Some(version) => self.api_version.as_deref() != Some(version),
        }
    }
}
#[derive(Clone)]
pub struct ChatCall {
    pub model: String,
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
