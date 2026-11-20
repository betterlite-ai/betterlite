use betterlite_context::non_default_params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::transcription::client::SharedSession;
use secrecy::SecretString;

// transcription's arguments after the signature defaults; the file travels beside the call
#[derive(Debug, Clone, Default)]
pub struct TranscriptionCall {
    pub model: String,
    pub language: Option<String>,
    pub prompt: Option<String>,
    pub response_format: Option<String>,
    pub timestamp_granularities: Option<Vec<String>>,
    pub temperature: Option<i64>,
    pub user: Option<String>,
    // None takes the signature's 600 seconds
    pub timeout: Option<f64>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub max_retries: Option<i64>,
    pub custom_llm_provider: Option<String>,
    pub base_url: Option<String>,
    // kwargs.pop("atranscription", False); only a literal true selects an async client
    pub atranscription: Option<Json>,
    // kwargs.pop("client", None), kept as the class the handler compares against
    pub client: Option<SuppliedClientKind>,
    pub extra_headers: Option<Headers>,
    // kwargs.get("shared_session", None), which the OpenAI and native handlers hand to their clients
    pub shared_session: Option<SharedSession>,
    // kwargs.pop("azure_ad_token", None), taken after the litellm_params snapshot
    pub azure_ad_token: Option<SecretString>,
    // the kwargs dict itself, which get_litellm_params and the optional params read
    pub params: Params,
}

// the OpenAI SDK client classes LiteLLM's handlers accept as a supplied client
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppliedClientKind {
    OpenAi,
    AsyncOpenAi,
    AzureOpenAi,
    AsyncAzureOpenAi,
    HttpHandler,
    AsyncHttpHandler,
}
