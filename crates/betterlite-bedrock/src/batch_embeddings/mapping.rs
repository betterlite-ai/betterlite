use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::error::BatchEmbeddingError;
use super::input::{EmbeddingInput, coerce_input_to_string};
use super::titan::is_titan_v2_embed_model;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::titan_embeddings::v2::{map_openai_params, transform_request};

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

// A request key kept apart as absent, null or a value, so only the keys the record carried reach the V2 mapper
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

// The Titan v2 InvokeModel modelInput, as B627's request constructor fills it
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvokeModelBody {
    pub input_text: String,
    // the outer Option is the key's presence and the inner one an explicit null, so a null stays in the body
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub dimensions: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_types: Option<Vec<EmbeddingType>>,
}

// The two values AWS accepts in embeddingTypes, which encoding_format maps onto
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingType {
    Binary,
    Float,
}

// The body minus model and input, as the V2 config reads it; a null value stays a null entry
fn non_default_params(request: &EmbeddingRequest) -> Object {
    let mut params = Object::new();
    if let Setting::Null = request.dimensions {
        params.insert(String::from("dimensions"), Json::Null);
    }
    if let Setting::Value(size) = request.dimensions {
        params.insert(String::from("dimensions"), Json::Number(Number::from(size)));
    }
    if let Setting::Null = &request.encoding_format {
        params.insert(String::from("encoding_format"), Json::Null);
    }
    if let Setting::Value(format) = &request.encoding_format {
        params.insert(String::from("encoding_format"), Json::Text(format.clone()));
    }
    params
}

// B627's param mapper and request constructor, read back into the typed InvokeModel body
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
    let inference_params =
        map_openai_params(&non_default_params(openai_request_body), Object::new());
    let transformed = transform_request(&input_text, &inference_params)?;
    let wire = serde_json::to_value(transformed).map_err(BatchEmbeddingError::Wire)?;
    serde_json::from_value(wire).map_err(BatchEmbeddingError::Wire)
}
