use thiserror::Error;

use crate::titan_embeddings::error::TitanEmbeddingsError;

#[derive(Debug, Error)]
pub enum BatchEmbeddingError {
    #[error("Embedding batch record is missing required `input` field: model={model}")]
    MissingInput { model: String },
    #[error(
        "Bedrock batch embedding requires one input per JSONL record. Got a list with {items} items for model={model}; emit one JSONL line per input string instead."
    )]
    MultipleInputs { items: usize, model: String },
    #[error(
        "Bedrock Titan v2 batch embedding does not support pre-tokenized integer inputs. Pass `input` as a string (model={model})."
    )]
    TokenizedInput { model: String },
    #[error(
        "Bedrock batch embedding `input` must be a string (or a single-element list of strings). Got type {found} for model={model}."
    )]
    InputNotString { found: &'static str, model: String },
    #[error(
        "Bedrock batch embedding currently supports only Amazon Titan Text Embeddings V2 (model id contains 'titan-embed-text-v2'). Got model='{model}'. Track other embedding models in https://github.com/BerriAI/litellm/issues."
    )]
    UnsupportedModel { model: String },
    #[error(transparent)]
    Titan(#[from] TitanEmbeddingsError),
    #[error("the shared Titan v2 request does not fit the InvokeModel body: {0}")]
    Wire(#[source] serde_json::Error),
}
