use std::collections::BTreeMap;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::Headers;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Supported { models: Vec<String> },
    ChatParams { cases: Vec<ChatParamsCase> },
    Chunk { cases: Vec<ChunkCase> },
    Embedding { cases: Vec<EmbeddingCase> },
    Responses { cases: Vec<ResponsesCase> },
    Decisions { cases: Vec<DecisionsCase> },
    ErrorClass { cases: Vec<ErrorCase> },
    Request { cases: Vec<RequestCase> },
    Response { cases: Vec<ReplyCase> },
    EmbeddingReply { cases: Vec<ReplyCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesCase {
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) global_api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) api_base: Option<String>,
    pub(crate) global_api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecisionsCase {
    pub(crate) cost: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCase {
    pub(crate) message: String,
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: Headers,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatParamsCase {
    pub(crate) model: String,
    pub(crate) requested: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkCase {
    pub(crate) chunk: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingCase {
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) api_base: Option<String>,
    pub(crate) model: String,
    pub(crate) input: EmbeddingInput,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) non_default_params: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCase {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) optional_params: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    pub(crate) body: String,
}
