use super::decode::{OpenRouterImageUsage, ReplyDecodeError};
use crate::api::image_contracts::records::{
    ImageHttpResponse, ImageInputTokens, ImageResponse, ImageUsage,
};
use crate::openai_base_config::Headers;
use crate::openrouter_chat::error::OpenRouterError;
pub use crate::openrouter_chat::error::get_error_class;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use std::io;
use thiserror::Error;
pub const SUPPORTED_PARAMS: [&str; 3] = ["size", "quality", "n"];
pub const SKIPPED_PARAMS: [&str; 3] = ["model", "messages", "modalities"];
pub const DEFAULT_API_BASE: &str = "https://openrouter.ai/api/v1";
pub const CHAT_COMPLETIONS: &str = "/chat/completions";
const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";
const SQUARE_ASPECT_RATIO: &str = "1:1";
#[derive(Debug, Error)]
pub enum ImageConfigError {
    #[error("size must be a string")]
    Size,
    #[error("quality must be a string")]
    Quality,
    #[error("image_config must be an object")]
    ImageConfig,
    #[error("OPENROUTER_API_KEY is not set")]
    MissingApiKey,
    #[error("Unsupported image type for OpenRouter image edit.")]
    UnsupportedImage,
    #[error("the image could not be read: {source}")]
    ImageRead {
        #[source]
        source: io::Error,
    },
    #[error("usage.cost must be a finite number")]
    Cost,
    #[error("usage.prompt_tokens minus its input image tokens does not fit in a whole number")]
    TextTokens,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageHiddenParams {
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub additional_headers: AdditionalHeaders,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_cost_details: Option<IndexMap<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<Json>,
}
#[must_use]
pub fn map_size_to_aspect_ratio(size: &str) -> &'static str {
    match size {
        "1536x1024" => "3:2",
        "1792x1024" => "16:9",
        "1024x1536" => "2:3",
        "1024x1792" => "9:16",
        _ => SQUARE_ASPECT_RATIO,
    }
}
#[must_use]
pub fn map_quality_to_image_size(quality: &str) -> Option<&'static str> {
    match quality {
        "low" | "standard" | "auto" => Some("1K"),
        "medium" => Some("2K"),
        "high" | "hd" => Some("4K"),
        _ => None,
    }
}
#[must_use]
pub fn resolved_key(candidates: [Option<&SecretString>; 3]) -> Option<&SecretString> {
    for key in candidates.into_iter().flatten() {
        if !key.expose_secret().is_empty() {
            return Some(key);
        }
    }
    None
}
#[must_use]
pub fn resolved_base(candidates: [Option<&str>; 2]) -> Option<&str> {
    for base in candidates.into_iter().flatten() {
        if !base.is_empty() {
            return Some(base);
        }
    }
    None
}
#[must_use]
pub fn authorization(key: Option<&SecretString>) -> String {
    match key {
        Some(secret) => format!("Bearer {}", secret.expose_secret()),
        None => String::from("Bearer None"),
    }
}
#[must_use]
pub fn transform_error(stage: &str, message: &str) -> OpenRouterError {
    OpenRouterError {
        message: format!("Error transforming OpenRouter {stage} response: {message}"),
        status_code: 500,
        headers: Headers::new(),
    }
}
#[must_use]
pub fn reply_error(
    error: ReplyDecodeError,
    raw: &ImageHttpResponse,
    stage: &str,
) -> OpenRouterError {
    match error {
        ReplyDecodeError::Json { source } => OpenRouterError {
            message: format!("Error parsing OpenRouter response: {source}"),
            status_code: i64::from(raw.status_code),
            headers: raw.headers.clone(),
        },
        ReplyDecodeError::Shape { source } => transform_error(stage, &source.to_string()),
    }
}
fn apply_cost(
    usage: &OpenRouterImageUsage,
    hidden: &mut ImageHiddenParams,
) -> Result<(), ImageConfigError> {
    if let Some(cost) = usage.cost {
        let amount = Number::from_f64(cost).ok_or(ImageConfigError::Cost)?;
        hidden
            .additional_headers
            .insert(String::from(COST_HEADER), Json::Number(amount));
    }
    if !usage.cost_details.is_empty() {
        hidden
            .response_cost_details
            .get_or_insert_with(IndexMap::new)
            .extend(usage.cost_details.clone());
    }
    Ok(())
}
pub fn apply_generation_usage(
    usage: &OpenRouterImageUsage,
    response: &mut ImageResponse,
    hidden: &mut ImageHiddenParams,
) -> Result<(), ImageConfigError> {
    response.usage = Some(ImageUsage {
        input_tokens: usage.prompt_tokens,
        input_tokens_details: ImageInputTokens {
            image_tokens: 0,
            text_tokens: usage.prompt_tokens,
        },
        output_tokens: usage.output_image_tokens,
        total_tokens: usage.total_tokens,
    });
    apply_cost(usage, hidden)
}
pub fn apply_edit_usage(
    usage: &OpenRouterImageUsage,
    response: &mut ImageResponse,
    hidden: &mut ImageHiddenParams,
) -> Result<(), ImageConfigError> {
    let text_tokens = usage
        .prompt_tokens
        .checked_sub(usage.input_image_tokens)
        .ok_or(ImageConfigError::TextTokens)?;
    response.usage = Some(ImageUsage {
        input_tokens: usage.prompt_tokens,
        input_tokens_details: ImageInputTokens {
            image_tokens: usage.input_image_tokens,
            text_tokens,
        },
        output_tokens: usage.output_image_tokens,
        total_tokens: usage.total_tokens,
    });
    apply_cost(usage, hidden)
}
