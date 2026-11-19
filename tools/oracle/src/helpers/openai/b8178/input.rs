use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    EmbeddingRequest(EmbeddingRequestStep),
    EmbeddingReply(EmbeddingReplyStep),
    PassthroughUrl(PassthroughUrlStep),
    PassthroughHeaders(PassthroughHeadersStep),
    PassthroughKey(PassthroughKeyStep),
    PassthroughStreaming(PassthroughStreamingStep),
    PassthroughLogging(PassthroughLoggingStep),
    PassthroughCollected(PassthroughCollectedStep),
}

#[derive(Debug, Deserialize)]
pub(crate) struct EmbeddingRequestStep {
    pub(crate) model: String,
    pub(crate) input: EmbeddingInput,
    pub(crate) api_base: Option<String>,
    pub(crate) env_base: Option<String>,
    pub(crate) token: String,
    #[serde(default)]
    pub(crate) headers: Headers,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EmbeddingReplyStep {
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PassthroughUrlStep {
    pub(crate) api_base: Option<String>,
    pub(crate) env_base: Option<String>,
    pub(crate) endpoint: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PassthroughHeadersStep {
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PassthroughKeyStep {
    pub(crate) api_key: Option<String>,
    pub(crate) env_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PassthroughStreamingStep {
    pub(crate) request_data: Object,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PassthroughLoggingStep {
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) status: u16,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PassthroughCollectedStep {
    pub(crate) chunks: Vec<String>,
    pub(crate) model: String,
    pub(crate) endpoint: String,
}
