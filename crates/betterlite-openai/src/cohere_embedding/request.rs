use std::slice;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::base64_string::is_base64_encoded;
use secrecy::{ExposeSecret as _, SecretString};

use super::{CohereEmbeddingError, DEFAULT_COMPLETE_URL, DEFAULT_EMBEDDING_INPUT_TYPE};
use crate::cohere_streams::cohere_error;
use crate::embedding::EmbeddingInput;
use crate::error::ProviderError;
use crate::openai_base_config::Headers;

#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        match name.as_str() {
            "encoding_format" => {
                let types = if let Json::List(items) = value {
                    items.clone()
                } else {
                    vec![value.clone()]
                };
                optional_params.insert(String::from("embedding_types"), Json::List(types));
            }
            "dimensions" => {
                optional_params.insert(String::from("output_dimension"), value.clone());
            }
            _ => {}
        }
    }
    optional_params
}

#[must_use]
pub fn validate_environment(headers: Headers, api_key: Option<&SecretString>) -> Headers {
    let mut merged = Headers::from([(
        String::from("Content-Type"),
        String::from("application/json"),
    )]);
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        merged.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    merged.extend(headers);
    merged
}

// CohereError carries only the status and the message, so the response headers are dropped
#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn get_error_class(
    error_message: String,
    status_code: u16,
    _headers: &Headers,
) -> ProviderError {
    cohere_error(status_code, error_message)
}

#[must_use]
pub fn get_complete_url(api_base: Option<&str>) -> String {
    match api_base {
        Some(base) if !base.is_empty() => base.to_owned(),
        Some(_) | None => String::from(DEFAULT_COMPLETE_URL),
    }
}

// _transform_request: the last input alone decides image versus text, and Bedrock callers reach it directly
#[must_use]
pub fn transform_request(model: &str, input: &[String], inference_params: &Object) -> Object {
    let image = input.last().is_some_and(|text| is_base64_encoded(text));
    let (rows_key, input_type) = if image {
        ("images", "image")
    } else {
        ("texts", DEFAULT_EMBEDDING_INPUT_TYPE)
    };
    let mut request = Object::new();
    request.insert(String::from("model"), Json::Text(model.to_owned()));
    request.insert(
        String::from(rows_key),
        Json::List(input.iter().cloned().map(Json::Text).collect()),
    );
    request.insert(
        String::from("input_type"),
        Json::Text(String::from(input_type)),
    );
    request.extend(inference_params.clone());
    request
}

// transform_embedding_request drops a token input at input[0] and wraps a single string in a list
pub fn transform_embedding_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Result<Object, CohereEmbeddingError> {
    let texts: &[String] = match input {
        EmbeddingInput::Text(text) => slice::from_ref(text),
        EmbeddingInput::Texts(texts) if texts.is_empty() => {
            return Err(CohereEmbeddingError::EmptyInput);
        }
        EmbeddingInput::Texts(texts) => texts,
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            return Err(CohereEmbeddingError::InputNotText);
        }
    };
    Ok(transform_request(model, texts, optional_params))
}
