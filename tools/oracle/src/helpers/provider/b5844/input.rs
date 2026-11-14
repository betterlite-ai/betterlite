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
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Url {
        model: String,
        embedding: bool,
        api_base: Option<String>,
        account_env: Option<String>,
        params: Object,
    },
    Headers {
        model: String,
        api_key: Option<String>,
        headers: Headers,
    },
    Params {
        model: String,
        embedding: bool,
        non_default: Object,
    },
    Info {
        api_base: Option<String>,
        api_key: Option<String>,
        jwt_env: Option<String>,
    },
    Request {
        model: String,
        messages: Vec<Message>,
        options: Object,
    },
    Response {
        model: String,
        body: String,
        request_data: Json,
        messages: Vec<Json>,
    },
    EmbeddingRequest {
        model: String,
        text: Json,
        options: Object,
    },
    EmbeddingResponse {
        model: String,
        body: String,
    },
    Chunk {
        chunk: ChunkJson,
    },
    Event {
        chunk: ChunkJson,
    },
}
