pub mod error;
pub mod reply;

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use self::error::OobaboogaError;
use super::error::LocalServerError;
use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, Mode, UrlDownloader};
use crate::embedding::EmbeddingInput;
use crate::openai_base_config::Headers;

const API_BASE_UNSET: &str = "API Base not set. Set one via completion(..,api_base='your-api-url')";

fn missing_api_base() -> OobaboogaError {
    OobaboogaError {
        status_code: 404,
        message: API_BASE_UNSET.to_owned(),
        headers: None,
    }
}

#[must_use]
pub fn validate_environment(api_key: Option<&SecretString>) -> Headers {
    let mut headers = Headers::from([
        (String::from("accept"), String::from("application/json")),
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
    ]);
    if let Some(key) = api_key {
        headers.insert(
            String::from("Authorization"),
            format!("Token {}", key.expose_secret()),
        );
    }
    headers
}

pub fn completion_url(model: &str, api_base: Option<&str>) -> Result<String, OobaboogaError> {
    let base = if model.starts_with("http://") || model.starts_with("https://") {
        model
    } else {
        api_base
            .filter(|base| !base.is_empty())
            .ok_or_else(missing_api_base)?
    };
    Ok(format!("{base}/v1/chat/completions"))
}

pub fn embeddings_url(model: &str, api_base: Option<&str>) -> Result<String, OobaboogaError> {
    if model.starts_with("http://") || model.starts_with("https://") {
        return Ok(model.to_owned());
    }
    let base = api_base
        .filter(|base| !base.is_empty())
        .ok_or_else(missing_api_base)?;
    Ok(format!("{base}/v1/embeddings"))
}

// the request body is OpenAIGPTConfig.transform_request with the provider name oobabooga
pub fn completion_body(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
    downloads: &dyn UrlDownloader,
) -> Result<Object, LocalServerError> {
    let environment = BTreeMap::new();
    let transformed = ChatContent {
        model,
        messages,
        optional_params,
        provider: Some("oobabooga"),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads,
    }
    .transform(Mode::Sync)?;
    Ok(transformed.to_body()?)
}

// optional_params are applied last, so they can replace the input key
pub fn embedding_body(
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Result<Object, LocalServerError> {
    let wire_input = serde_json::to_value(input)
        .and_then(serde_json::from_value::<Json>)
        .map_err(|source| LocalServerError::Input { source })?;
    let mut body = Object::from_iter([(String::from("input"), wire_input)]);
    body.extend(optional_params.clone());
    Ok(body)
}
