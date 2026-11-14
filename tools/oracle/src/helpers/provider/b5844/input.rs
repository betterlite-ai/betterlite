use betterlite_models::decisions::Json as ChunkJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_provider::api::file_contracts::config::Headers;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Url(UrlStep),
    Headers(HeadersStep),
    Params(ParamsStep),
    Info(InfoStep),
    Request(RequestStep),
    Response(ResponseStep),
    EmbeddingRequest(EmbeddingRequestStep),
    EmbeddingResponse(EmbeddingResponseStep),
    Chunk(ChunkStep),
    Event(ChunkStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    pub(crate) model: String,
    pub(crate) embedding: bool,
    pub(crate) api_base: Option<String>,
    pub(crate) account_env: Option<String>,
    pub(crate) params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadersStep {
    pub(crate) model: String,
    pub(crate) api_key: Option<String>,
    pub(crate) headers: Headers,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsStep {
    pub(crate) model: String,
    pub(crate) embedding: bool,
    pub(crate) non_default: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InfoStep {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) jwt_env: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestStep {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) options: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseStep {
    pub(crate) model: String,
    pub(crate) body: String,
    pub(crate) request_data: Json,
    pub(crate) messages: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingRequestStep {
    pub(crate) model: String,
    pub(crate) text: Json,
    pub(crate) options: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingResponseStep {
    pub(crate) model: String,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkStep {
    pub(crate) chunk: ChunkJson,
}
