use betterlite_streaming::provider_chunk_decode::chunk::ProviderKind;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamOptions};

// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default; the caller resolves the env override.
pub const REPEATED_CHUNK_LIMIT: usize = 100;

// Which iterator __init__ was built for: __next__ or __anext__.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallMode {
    Blocking,
    Async,
}

pub struct StreamLoggingFlags {
    pub disable: bool,
}

// The settings CustomStreamWrapper works out in __init__, before the first chunk arrives.
pub struct ChatStreamConfig {
    pub model: String,
    pub provider: Option<String>,
    pub logging_provider: Option<String>,
    pub stream_options: Option<StreamOptions>,
    pub send_stream_usage: bool,
    pub merge_reasoning_in_choices: bool,
    pub optional_params: Option<JsonObject>,
    pub hidden_params: JsonObject,
    // _hidden_params["mcp_metadata"], which the MCP handler adds after __init__ so the base snapshot lacks it.
    pub mcp_metadata: Option<JsonObject>,
    pub max_streaming_duration: Option<f64>,
    pub repeated_chunk_limit: usize,
    pub mode: CallMode,
    pub provider_kind: ProviderKind,
    pub completion_stream: Option<String>,
    pub logging: StreamLoggingFlags,
}

impl ChatStreamConfig {
    // __init__'s _effective_model: an openai-compatible call names the logging provider in the model.
    #[must_use]
    pub fn effective_model(&self) -> String {
        if self.provider.as_deref() != Some("openai") || self.provider == self.logging_provider {
            return self.model.clone();
        }
        let logged = self.logging_provider.as_deref().unwrap_or("None");
        format!("{logged}/{}", self.model)
    }

    // The snapshot taken after __init__: the hidden params every chunk reports, with no cost yet.
    #[must_use]
    pub fn base_hidden_params(&self) -> JsonObject {
        let mut hidden = self.hidden_params.clone();
        hidden.insert("response_cost".to_owned(), JsonValue::Null);
        // model_response_creator stamps the logging object's provider here, and the base spread wins.
        let provider = self
            .logging_provider
            .as_ref()
            .map_or(JsonValue::Null, |name| JsonValue::String(name.clone()));
        hidden.insert("custom_llm_provider".to_owned(), provider);
        hidden
    }
}
