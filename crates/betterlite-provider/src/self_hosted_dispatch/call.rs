use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use betterlite_openai::compatible_embedding::EmbeddingCall as SharedEmbeddingCall;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

// the provider config a self-hosted arm calls, as main.py's custom_llm_provider string picks it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfHostedConfig {
    HostedVllm,
    Lemonade,
    Ollama,
    OllamaChat,
    Oobabooga,
    Petals,
    Triton,
}

#[derive(Debug, Clone)]
pub struct CompletionCall {
    pub config: SelfHostedConfig,
    pub model: String,
    pub stream: Option<bool>,
    pub acompletion: bool,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub custom_llm_provider: String,
    pub headers: Option<BTreeMap<String, String>>,
    pub litellm_params: Object,
    pub optional_params: Object,
    pub messages: Vec<Message>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct EmbeddingCall {
    pub model: String,
    pub input: EmbeddingInput,
    pub optional_params: Object,
    pub litellm_params: Object,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub timeout: Duration,
    pub aembedding: bool,
}

// the embedding provider config an arm calls, as get_provider_embedding_config picks it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    HostedVllm,
    Triton,
    Oobabooga,
    Ollama,
    Xinference,
    Llamafile,
    LmStudio,
}

// ollama's embedding request: its own base and the prompts its input turned into
#[derive(Debug, Clone)]
pub struct OllamaRequest {
    pub api_base: String,
    pub model: String,
    pub prompts: Vec<String>,
    pub optional_params: Object,
}

// what an embedding arm hands on: the values its provider config reads, or B1440's prepared shared call
pub enum EmbeddingRequest {
    Compatible {
        provider: EmbeddingProvider,
        api_base: Option<String>,
        api_key: Option<SecretString>,
        input: EmbeddingInput,
        optional_params: Object,
        headers: Option<BTreeMap<String, String>>,
    },
    Ollama(OllamaRequest),
    Shared {
        provider: EmbeddingProvider,
        call: Box<SharedEmbeddingCall>,
    },
}

// B1440's EmbeddingCall carries no Debug, so the shared arm reports the fields that identify it
impl fmt::Debug for EmbeddingRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compatible {
                provider, api_base, ..
            } => f
                .debug_struct("Compatible")
                .field("provider", provider)
                .field("api_base", api_base)
                .finish_non_exhaustive(),
            Self::Ollama(request) => f
                .debug_struct("Ollama")
                .field("api_base", &request.api_base)
                .field("model", &request.model)
                .finish_non_exhaustive(),
            Self::Shared { provider, call } => f
                .debug_struct("Shared")
                .field("provider", provider)
                .field("model", &call.model)
                .field("api_base", &call.api_base)
                .finish_non_exhaustive(),
        }
    }
}
