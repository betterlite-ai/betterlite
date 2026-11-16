use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::vertex_embedding_dispatch::{Native, Resolution};
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_vertex::gemini_embedding_records::input::{
    EmbeddingInput as EmbedContentInput, InputEntry,
};
use secrecy::SecretString;
use serde_json::Number;

use super::{VertexEmbeddingInput, VertexEmbeddingPrepareError, VertexEmbeddingPrepared};

// main.py's batch_embeddings call: api_key=None and no timeout, with the caller's headers
#[derive(Debug, Clone)]
pub struct EmbedContentArguments {
    pub model: String,
    pub aembedding: bool,
    pub resolved: Resolution,
    pub input: EmbedContentInput,
    pub extra_headers: BTreeMap<String, String>,
}

// main.py's multimodal_embedding call passes litellm_params but no timeout, headers or api_key
#[derive(Debug, Clone)]
pub struct MultimodalArguments {
    pub model: String,
    pub aembedding: bool,
    pub resolved: Resolution,
    pub input: Json,
    pub litellm_params: Option<Extra>,
}

// main.py's vertex_embedding.embedding call: the timeout and api_key, no headers
#[derive(Debug, Clone)]
pub struct TextArguments {
    pub model: String,
    pub aembedding: bool,
    pub resolved: Resolution,
    pub input: EmbeddingInput,
    pub litellm_params: Option<Extra>,
    pub api_key: Option<SecretString>,
    pub timeout: Option<Duration>,
}

pub(super) fn arguments(
    request: VertexEmbeddingInput,
    resolved: Resolution,
) -> Result<VertexEmbeddingPrepared, VertexEmbeddingPrepareError> {
    let prepared = match resolved.native {
        Native::EmbedContent => {
            VertexEmbeddingPrepared::EmbedContent(Box::new(EmbedContentArguments {
                input: embed_content_input(&request.input)?,
                extra_headers: request.extra_headers,
                model: request.model,
                aembedding: request.aembedding,
                resolved,
            }))
        }
        Native::Multimodal => VertexEmbeddingPrepared::Multimodal(Box::new(MultimodalArguments {
            input: input_json(&request.input),
            litellm_params: request.litellm_params,
            model: request.model,
            aembedding: request.aembedding,
            resolved,
        })),
        Native::Text => VertexEmbeddingPrepared::Text(Box::new(TextArguments {
            input: request.input,
            litellm_params: request.litellm_params,
            api_key: request.api_key,
            timeout: request.timeout,
            model: request.model,
            aembedding: request.aembedding,
            resolved,
        })),
    };
    Ok(prepared)
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

fn token_list(tokens: &[u32]) -> Json {
    let numbers = tokens
        .iter()
        .map(|token| Json::Number(Number::from(*token)));
    Json::List(numbers.collect())
}

fn input_json(input: &EmbeddingInput) -> Json {
    match input {
        EmbeddingInput::Text(text) => Json::Text(text.clone()),
        EmbeddingInput::Texts(texts) => Json::List(texts.iter().cloned().map(Json::Text).collect()),
        EmbeddingInput::Tokens(tokens) => token_list(tokens),
        EmbeddingInput::TokenLists(lists) => {
            Json::List(lists.iter().map(|tokens| token_list(tokens)).collect())
        }
    }
}
