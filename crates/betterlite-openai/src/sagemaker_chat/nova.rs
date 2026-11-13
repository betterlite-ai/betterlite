use betterlite_prompts::arguments::json::Object as Body;

use crate::chat_content::ChatContentError;
use crate::openai_base_config::CatalogCapabilities;

use super::request::supported_openai_params as chat_supported_openai_params;
use super::request::{TransformInput, build_body};

pub const PROVIDER: &str = "sagemaker_nova";
// Nova shares the sagemaker_chat stream wrapper and, unlike it, expects stream in the body
pub const HAS_CUSTOM_STREAM_WRAPPER: bool = true;
pub const SUPPORTS_STREAM_PARAM_IN_BODY: bool = true;

const NOVA_PARAMS: [&str; 4] = [
    "top_k",
    "reasoning_effort",
    "allowed_token_ids",
    "truncate_prompt_tokens",
];

#[must_use]
pub fn supported_openai_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
    let mut params = chat_supported_openai_params(model, catalog);
    for name in NOVA_PARAMS {
        if !params.contains(&name) {
            params.push(name);
        }
    }
    params
}

pub fn transform_request(input: &TransformInput<'_>) -> Result<Body, ChatContentError> {
    let mut body = build_body(input, PROVIDER)?;
    // Nova endpoints reject model in the body, so the override above is dropped with it
    body.shift_remove("model");
    Ok(body)
}
