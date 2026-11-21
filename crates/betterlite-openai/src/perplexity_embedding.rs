pub mod error;
pub mod reply;
mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::embedding::EmbeddingInput;
use crate::error::{ErrorRequest, ErrorResponse, ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;
use error::PerplexityEmbeddingError;

pub const DEFAULT_URL: &str = "https://api.perplexity.ai/v1/embeddings";
const API_KEY_ENVS: [&str; 2] = ["PERPLEXITYAI_API_KEY", "PERPLEXITY_API_KEY"];

// drop_params never matters here: an unsupported name is skipped whatever the flag says
pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["dimensions", "encoding_format"];

// the base is used as given, so a base ending in /v1 becomes /v1/v1/embeddings
#[must_use]
pub fn get_complete_url(api_base: Option<&str>) -> String {
    match api_base.filter(|base| !base.is_empty()) {
        Some(base) if base.ends_with("/embeddings") => base.to_owned(),
        Some(base) => format!("{base}/v1/embeddings"),
        None => DEFAULT_URL.to_owned(),
    }
}

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// LiteLLM builds a fresh map, so no caller header reaches the wire, and a missing key prints as None
#[must_use]
pub fn validate_environment(
    api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> Headers {
    let key = api_key.map_or_else(
        || {
            API_KEY_ENVS
                .iter()
                .find_map(|name| environment.lookup(name).filter(|value| !value.is_empty()))
                .unwrap_or_else(|| String::from("None"))
        },
        |key| key.expose_secret().to_owned(),
    );
    Headers::from([
        (String::from("Authorization"), format!("Bearer {key}")),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ])
}

// the optional params spread last, so one named model or input replaces the request's own
pub fn transform_embedding_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Result<Object, PerplexityEmbeddingError> {
    let input_json = serde_json::to_value(input)
        .and_then(serde_json::from_value::<Json>)
        .map_err(|source| PerplexityEmbeddingError::Encode { source })?;
    let mut body = Object::from([
        (String::from("model"), Json::Text(model.to_owned())),
        (String::from("input"), input_json),
    ]);
    body.extend(optional_params.clone());
    Ok(body)
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> ProviderError {
    let request = ErrorRequest {
        method: String::from("POST"),
        url: DEFAULT_URL.to_owned(),
    };
    ProviderError::from(ProviderErrorParts {
        status_code,
        message: message.to_owned(),
        headers: Some(headers.clone().into_iter().collect()),
        request: Some(request.clone()),
        response: Some(ErrorResponse {
            status_code,
            request,
        }),
        body: None,
        status_code_is_synthesized: false,
    })
}
