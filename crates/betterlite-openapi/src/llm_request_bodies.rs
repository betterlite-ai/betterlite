use serde::{Deserialize, Serialize};

use crate::schema_transforms::add_request_schema;
use crate::schema_transforms::tree::JsonObject;

mod tests;

const CHAT_COMPLETION_PATHS: [&str; 4] = [
    "/v1/chat/completions",
    "/chat/completions",
    "/engines/{model}/chat/completions",
    "/openai/deployments/{model}/chat/completions",
];

const EMBEDDING_PATHS: [&str; 4] = [
    "/v1/embeddings",
    "/embeddings",
    "/engines/{model}/embeddings",
    "/openai/deployments/{model}/embeddings",
];

const RESPONSES_API_PATHS: [&str; 3] = ["/v1/responses", "/responses", "/openai/v1/responses"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmRequestSchemas {
    pub chat_completion: Option<JsonObject>,
    pub embedding: Option<JsonObject>,
    pub responses: Option<JsonObject>,
}

pub fn add_llm_request_bodies(document: &mut JsonObject, schemas: &LlmRequestSchemas) {
    add_request_schema(
        document,
        "ProxyChatCompletionRequest",
        schemas.chat_completion.as_ref(),
        &CHAT_COMPLETION_PATHS,
    );
    add_request_schema(
        document,
        "EmbeddingRequest",
        schemas.embedding.as_ref(),
        &EMBEDDING_PATHS,
    );
    add_request_schema(
        document,
        "ResponsesAPIRequestParams",
        schemas.responses.as_ref(),
        &RESPONSES_API_PATHS,
    );
}
