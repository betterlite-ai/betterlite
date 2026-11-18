use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_config::sections::openai_provider::Settings as ProviderSettings;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers, Object, OpenAiSettings};
use betterlite_openai::text_completion_policy::TextCompletionSettings;
use indexmap::IndexMap;
use secrecy::SecretString;

// The configuration a composition call reads; every value is resolved once at startup, so a call never rereads the environment.
pub struct Composition {
    pub credentials: OpenAiCredentialsSettings,
    pub provider: ProviderSettings,
    pub globals: LiteLlmGlobals,
    // litellm.headers, which the dispatcher resolves beside the call's own headers
    pub globals_headers: Option<Headers>,
    pub env_api_key: Option<SecretString>,
    pub chat_defaults: OpenAiSettings,
    pub text_defaults: TextCompletionSettings,
    pub catalog: CatalogCapabilities,
}

// The per-call values a route resolved from its own request; none of them is shared with another deployment.
#[derive(Debug, Default)]
pub struct Credentials {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
}

// What the provider chain resolves to for one call, so a second call cannot reuse the first call's identity.
#[derive(Debug)]
pub struct Resolved {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
}

pub enum OpenAiOperation {
    Chat(Box<ChatCall>),
    Embedding(Box<EmbeddingCall>),
    Text(Box<TextCall>),
}

// The chat branch's call: the request body already parsed, plus the headers and metadata the dispatcher reads.
pub struct ChatCall {
    pub model: String,
    pub request: ChatRequest,
    pub credentials: Credentials,
    // completion() merges the caller's headers and extra_headers into this map before dispatch, and the SDK receives extra_headers again as request options
    pub headers: Option<Headers>,
    pub extra_headers: Option<Headers>,
    pub timeout: Option<f64>,
    pub metadata: Option<IndexMap<String, JsonValue>>,
}

// The embedding branch's call; encoding_format stays raw text because the resolution accepts any requested format.
pub struct EmbeddingCall {
    pub model: String,
    pub request: EmbeddingRequest,
    pub encoding_format: Option<String>,
    pub credentials: Credentials,
    pub headers: Option<Headers>,
    pub timeout: Option<f64>,
}

// The text branch's call: the messages B2237 transforms into a prompt and the options the call already carries.
pub struct TextCall {
    pub model: String,
    pub messages: Vec<JsonValue>,
    pub optional_params: Object,
    pub credentials: Credentials,
    pub headers: Option<Headers>,
    pub timeout: Option<f64>,
    pub acompletion: bool,
    pub text_completion: bool,
}
