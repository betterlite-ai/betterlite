use std::collections::BTreeMap;

use betterlite_azure::azure_chat_base_policy::AzureChatSettings;
use betterlite_azure::endpoints::cloudflare::CloudflareParams;
use betterlite_azure::endpoints::projection::CallableProvider;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{Headers, OpenAiSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

// litellm's module globals: the ranks read them after the call's own values and before the environment
#[derive(Debug, Default)]
pub struct Globals {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub azure_default_api_version: Option<String>,
}

// The configuration a dispatch reads; every value is resolved once at startup, so a call never rereads it.
pub struct Composition {
    pub environment: BTreeMap<String, String>,
    pub globals: Globals,
    // litellm.AzureOpenAIConfig(), the B684 defaults merged for a non-o-series model
    pub chat_defaults: AzureChatSettings,
    // litellm.AzureOpenAIO1Config(), the B532 defaults merged for an o-series model
    pub o_series_defaults: OpenAiSettings,
    // litellm_settings.openai_system_messages_first
    pub openai_system_messages_first: bool,
}

const AZURE_ENVIRONMENT: [&str; 7] = [
    "AZURE_API_BASE",
    "AZURE_API_VERSION",
    "AZURE_API_TYPE",
    "AZURE_OPENAI_API_KEY",
    "AZURE_API_KEY",
    "AZURE_AD_TOKEN",
    "AZURE_DEFAULT_API_VERSION",
];

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

    // The adapter has no configuration handle, so the process environment is the only source it can read.
    #[must_use]
    pub fn process() -> Self {
        let mut environment = BTreeMap::new();
        for name in AZURE_ENVIRONMENT {
            if let Ok(value) = std::env::var(name) {
                environment.insert(name.to_owned(), value);
            }
        }
        Self::from_environment(environment)
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&str> {
        self.environment.get(name).map(String::as_str)
    }
}

// The per-call values a route resolved from its own request; none of them is shared with another deployment.
#[derive(Clone, Default)]
pub struct Credentials {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub extra_headers: Option<Headers>,
    // kwargs.max_retries: chat inserts it when it is not None, text never inserts it
    pub max_retries: Option<Json>,
    // kwargs.extra_body, whose azure_ad_token is popped before any lookup
    pub extra_body: Option<Json>,
    pub azure_ad_token: Option<SecretString>,
    pub azure_ad_token_provider: Option<CallableProvider>,
    pub base_model: Option<String>,
    pub timeout: Option<f64>,
    pub cloudflare_params: CloudflareParams,
}

// A supplied SDK client, whose own api_version is the custom query it was built with.
#[derive(Debug, Clone)]
pub struct SuppliedClient {
    pub api_version: Option<String>,
}

impl SuppliedClient {
    // _check_dynamic_azure_params: only a non-None version that differs from the client's discards it
    #[must_use]
    pub fn discarded(&self, api_version: Option<&str>) -> bool {
        api_version.is_some_and(|version| self.api_version.as_deref() != Some(version))
    }
}

// A chat call: the parsed messages plus the options the caller wrote.
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
