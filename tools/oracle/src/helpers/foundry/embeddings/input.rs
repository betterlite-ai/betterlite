use betterlite_foundry::embeddings::request::FoundryEmbeddingOptions;
use betterlite_http::client_kind::ClientKind;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) flows: Vec<FlowCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FlowCall {
    pub(super) mode: ClientKind,
    pub(super) model: String,
    pub(super) input: Vec<String>,
    #[serde(default)]
    pub(super) options: OptionsCall,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    #[serde(default = "default_timeout")]
    pub(super) timeout: u64,
    pub(super) image_reply: ReplyCall,
    pub(super) text_reply: ReplyCall,
    #[serde(default)]
    pub(super) client: ClientName,
}

// which client the caller hands the route: none, an OpenAI SDK client, or the handler the route reuses
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ClientName {
    #[default]
    None,
    #[serde(rename = "openai")]
    OpenAi,
    Handler,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct OptionsCall {
    pub(super) input_type: Option<String>,
    pub(super) truncate: Option<String>,
    #[serde(flatten)]
    pub(super) passthrough: Object,
}

impl OptionsCall {
    pub(super) fn options(&self) -> FoundryEmbeddingOptions {
        FoundryEmbeddingOptions {
            input_type: self.input_type.clone(),
            truncate: self.truncate.clone(),
            passthrough: self.passthrough.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyCall {
    #[serde(default = "default_status")]
    pub(super) status: u16,
    #[serde(default)]
    pub(super) headers: Vec<HeaderCall>,
    pub(super) body: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HeaderCall {
    pub(super) name: String,
    pub(super) value: String,
}

const fn default_timeout() -> u64 {
    5
}

const fn default_status() -> u16 {
    200
}
