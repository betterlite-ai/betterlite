use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::vertex_embedding_dispatch::Resolution;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_vertex::gemini_embedding_records::input::{
    EmbeddingInput as EmbedContentInput, InputEntry,
};
use secrecy::SecretString;
use serde_json::Number;

use super::{VertexEmbeddingInput, VertexEmbeddingPrepareError};

// main.py's batch_embeddings call: api_key=None and no timeout, with the caller's headers
#[derive(Debug, Clone)]
pub struct EmbedContentArguments {
    pub model: String,
    pub input: EmbedContentInput,
    pub optional_params: Extra,
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub aembedding: bool,
}

// main.py's multimodal_embedding call passes litellm_params but no timeout, headers or api_key
#[derive(Debug, Clone)]
pub struct MultimodalArguments {
    pub model: String,
    pub input: Json,
    pub optional_params: Extra,
    pub litellm_params: Option<Extra>,
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
    pub aembedding: bool,
}

// main.py's vertex_embedding.embedding call: the timeout and api_key, no headers
#[derive(Debug, Clone)]
pub struct TextArguments {
    pub model: String,
    pub input: EmbeddingInput,
    pub optional_params: Json,
    pub litellm_params: Option<Json>,
    pub api_key: Option<SecretString>,
    pub timeout: Option<Duration>,
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
    pub aembedding: bool,
}

pub(super) fn embed_content_arguments(
    request: VertexEmbeddingInput,
    resolution: Resolution,
) -> Result<EmbedContentArguments, VertexEmbeddingPrepareError> {
    Ok(EmbedContentArguments {
        input: embed_content_input(&request.input)?,
        model: request.model,
        optional_params: resolution.optional_params,
        project: resolution.project,
        location: resolution.location,
        credentials: resolution.credentials,
        api_base: resolution.api_base,
        extra_headers: request.extra_headers,
        aembedding: request.aembedding,
    })
}

pub(super) fn multimodal_arguments(
    request: VertexEmbeddingInput,
    resolution: Resolution,
) -> MultimodalArguments {
    MultimodalArguments {
        input: input_json(&request.input),
        model: request.model,
        optional_params: resolution.optional_params,
        litellm_params: request.litellm_params,
        project: resolution.project,
        location: resolution.location,
        credentials: resolution.credentials,
        api_base: resolution.api_base,
        aembedding: request.aembedding,
    }
}

pub(super) fn text_arguments(
    request: VertexEmbeddingInput,
    resolution: Resolution,
) -> TextArguments {
    TextArguments {
        model: request.model,
        input: request.input,
        optional_params: Json::Object(resolution.optional_params),
        litellm_params: request.litellm_params.map(Json::Object),
        api_key: request.api_key,
        timeout: request.timeout,
        project: resolution.project,
        location: resolution.location,
        credentials: resolution.credentials,
        api_base: resolution.api_base,
        aembedding: request.aembedding,
    }
}

// LiteLLM's embedContent path raises for integer input, and the B1341 call has no form for it
fn embed_content_input(
    input: &EmbeddingInput,
) -> Result<EmbedContentInput, VertexEmbeddingPrepareError> {
    match input {
        EmbeddingInput::Text(text) => Ok(EmbedContentInput::Text(text.clone())),
        EmbeddingInput::Texts(texts) => Ok(EmbedContentInput::Entries(
            texts.iter().cloned().map(InputEntry::Text).collect(),
        )),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            Err(VertexEmbeddingPrepareError::TokenInput)
        }
    }
}

fn input_json(input: &EmbeddingInput) -> Json {
    match input {
        EmbeddingInput::Text(text) => Json::Text(text.clone()),
        EmbeddingInput::Texts(texts) => Json::List(texts.iter().cloned().map(Json::Text).collect()),
        EmbeddingInput::Tokens(tokens) => Json::List(
            tokens
                .iter()
                .copied()
                .map(Number::from)
                .map(Json::Number)
                .collect(),
        ),
        EmbeddingInput::TokenLists(lists) => Json::List(
            lists
                .iter()
                .map(|list| {
                    Json::List(
                        list.iter()
                            .copied()
                            .map(Number::from)
                            .map(Json::Number)
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}
