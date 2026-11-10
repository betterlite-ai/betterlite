pub mod error;
pub mod request;
pub mod response;
pub mod task;
pub mod url;

mod tests;

use secrecy::SecretString;

use crate::huggingface_chat::HuggingfaceEnv;
use crate::huggingface_chat::headers::validate_environment;
use crate::openai_base_config::Headers;

use error::HuggingfaceEmbeddingError;
use request::Params;
use task::{PipelineTagSource, resolve_task};

pub struct PrepareRequest<'request> {
    pub model: &'request str,
    pub input: &'request [String],
    pub optional_params: &'request Params,
    pub headers: &'request Headers,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub env: &'request HuggingfaceEnv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequest {
    pub url: String,
    pub headers: Headers,
    pub body: String,
}

pub fn prepare(
    request: &PrepareRequest<'_>,
    source: &dyn PipelineTagSource,
) -> Result<PreparedRequest, HuggingfaceEmbeddingError> {
    let headers = validate_environment(request.headers, request.api_key);
    let task = resolve_task(
        request.model,
        request.optional_params.get("input_type"),
        source,
    )?;
    let url = url::embedding_url(
        request.model,
        request.api_base,
        request.env,
        task.as_deref(),
    );
    let data = request::transform_input(
        request.model,
        request.input,
        request.optional_params,
        source,
    )?;
    let body = serde_json::to_string(&data)
        .map_err(|error| HuggingfaceEmbeddingError::RequestEncode { source: error })?;
    Ok(PreparedRequest { url, headers, body })
}
