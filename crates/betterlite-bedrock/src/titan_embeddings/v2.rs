use betterlite_openai::embedding::EmbeddingVector;

use betterlite_prompts::arguments::json::{Json, Object};

use super::error::TitanEmbeddingsError;
use super::response;

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["dimensions", "encoding_format"];

#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        match name.as_str() {
            "dimensions" => {
                optional_params.insert(String::from("dimensions"), value.clone());
            }
            "encoding_format" => {
                let encoded = if value.as_str() == Some("base64") {
                    "binary"
                } else {
                    "float"
                };
                optional_params.insert(
                    String::from("embeddingTypes"),
                    Json::List(vec![Json::Text(String::from(encoded))]),
                );
            }
            _ => {}
        }
    }
    optional_params
}

// AmazonTitanV2EmbeddingRequest(inputText=input, **inference_params), whose Python call rejects a second inputText
pub fn transform_request(
    input: &str,
    inference_params: &Object,
) -> Result<Object, TitanEmbeddingsError> {
    if inference_params.contains_key("inputText") {
        return Err(TitanEmbeddingsError::KeywordCollision("inputText"));
    }
    let mut request = Object::new();
    request.insert(String::from("inputText"), Json::Text(input.to_owned()));
    request.extend(inference_params.clone());
    Ok(request)
}

// embeddingsByType.binary wins by key presence, then embeddingsByType.float, then the legacy embedding column
pub fn by_type_embedding(reply: &Object) -> Result<EmbeddingVector, TitanEmbeddingsError> {
    if let Some(by_type) = reply.get("embeddingsByType")
        && let Some(value) = selected_by_type(by_type)?
    {
        return response::embedding_vector(value, "embeddingsByType");
    }
    let Some(embedding) = reply.get("embedding") else {
        return Err(TitanEmbeddingsError::NoEmbeddingData(Json::Object(
            reply.clone(),
        )));
    };
    response::embedding_vector(embedding, "embedding")
}

fn selected_by_type(by_type: &Json) -> Result<Option<&Json>, TitanEmbeddingsError> {
    match by_type {
        Json::Object(fields) => Ok(fields.get("binary").or_else(|| fields.get("float"))),
        Json::Text(text) => {
            if text.contains("binary") || text.contains("float") {
                return Err(TitanEmbeddingsError::NotSubscriptable);
            }
            Ok(None)
        }
        Json::List(items) => {
            if items.iter().any(selected_name) {
                return Err(TitanEmbeddingsError::NotSubscriptable);
            }
            Ok(None)
        }
        Json::Null | Json::Bool(_) | Json::Number(_) => Err(TitanEmbeddingsError::NotIterable),
    }
}

fn selected_name(item: &Json) -> bool {
    matches!(item.as_str(), Some("binary" | "float"))
}
