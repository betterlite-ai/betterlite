use serde::{Deserialize, Serialize};

use super::error::BatchEmbeddingError;
use super::input::{EmbeddingInput, coerce_input_to_string};
use super::titan::is_titan_v2_embed_model;
use crate::bedrock_model_capabilities::ModelInfoLookup;

// An OpenAI /v1/embeddings body, with the params the Titan v2 mapping reads and nothing else
#[derive(Debug, Deserialize)]
pub struct EmbeddingRequest {
    #[serde(default)]
    pub input: Option<EmbeddingInput>,
    #[serde(default)]
    pub dimensions: Setting<i64>,
    #[serde(default)]
    pub encoding_format: Setting<String>,
}

// A request key kept apart as absent, null or a value: the Titan body drops it, keeps a null, or reads a value
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(from = "Option<T>")]
pub enum Setting<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}

impl<T> From<Option<T>> for Setting<T> {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Self::Value)
    }
}

// The Titan v2 InvokeModel modelInput: inputText plus whichever inference params the request set
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvokeModelBody {
    pub input_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<Dimensions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_types: Option<Vec<EmbeddingType>>,
}

// A dimensions the request set: null stays null in the body and an integer carries its size
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Dimensions {
    Value(i64),
    Null,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingType {
    Binary,
    Float,
}

// An OpenAI /v1/embeddings body as the Titan v2 InvokeModel modelInput; every other embedding model is refused first
pub async fn map_embedding_to_bedrock<Lookup>(
    openai_request_body: &EmbeddingRequest,
    model: &str,
    lookup: &Lookup,
) -> Result<InvokeModelBody, BatchEmbeddingError>
where
    Lookup: ModelInfoLookup,
{
    if !is_titan_v2_embed_model(model, lookup).await {
        return Err(BatchEmbeddingError::UnsupportedModel {
            model: model.to_owned(),
        });
    }
    let input_text = coerce_input_to_string(openai_request_body.input.as_ref(), model)?;
    let dimensions = match openai_request_body.dimensions {
        Setting::Absent => None,
        Setting::Null => Some(Dimensions::Null),
        Setting::Value(size) => Some(Dimensions::Value(size)),
    };
    // a present encoding_format always asks for embeddingTypes, and only base64 leaves the float default
    let embedding_types = match &openai_request_body.encoding_format {
        Setting::Absent => None,
        Setting::Null => Some(vec![EmbeddingType::Float]),
        Setting::Value(format) => Some(vec![embedding_type(format)]),
    };
    Ok(InvokeModelBody {
        input_text,
        dimensions,
        embedding_types,
    })
}

// base64 asks Bedrock for binary embeddings; LiteLLM reads every other value as float
fn embedding_type(format: &str) -> EmbeddingType {
    if format == "base64" {
        EmbeddingType::Binary
    } else {
        EmbeddingType::Float
    }
}
