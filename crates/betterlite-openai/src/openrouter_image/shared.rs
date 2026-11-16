use std::io;

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use thiserror::Error;

use crate::api::image_contracts::records::{
    ImageHttpResponse, ImageInputTokens, ImageResponse, ImageUsage,
};
use crate::openai_base_config::Headers;
use crate::openrouter_chat::error::OpenRouterError;

// LiteLLM's get_error_class on both image configs is the same pass-through to OpenRouterException
pub use crate::openrouter_chat::error::get_error_class;

use super::decode::{OpenRouterImageUsage, ReplyDecodeError};

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
    #[error("{path} must be a whole number")]
    UsageCount { path: &'static str },
    #[error("{path} must be an object")]
    UsageDetails { path: &'static str },
    #[error("usage.cost must be a finite number")]
    Cost,
    #[error("usage.cost_details must be an object")]
    CostDetails,
    #[error("usage.prompt_tokens minus its input image tokens does not fit in a whole number")]
    TextTokens,
}

// LiteLLM's ImageResponse._hidden_params: a header map, the merged cost details and the reply's model
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageHiddenParams {
    pub additional_headers: AdditionalHeaders,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_cost_details: Option<IndexMap<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<Json>,
}

// LiteLLM's two tables; a size the table misses still gets the square ratio
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

// an unknown quality maps to no image size at all
#[must_use]
pub fn map_quality_to_image_size(quality: &str) -> Option<&'static str> {
    match quality {
        "low" | "standard" | "auto" => Some("1K"),
        "medium" => Some("2K"),
        "high" | "hd" => Some("4K"),
        _ => None,
    }
}

// LiteLLM's "api_key or litellm.api_key or OPENROUTER_API_KEY", with an empty string counting as unset
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

// a 500 with no headers, raised for every failure after the reply's JSON was parsed
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
        ReplyDecodeError::Shape { message } => transform_error(stage, message),
    }
}

fn whole_number(value: Option<&Json>, path: &'static str) -> Result<i64, ImageConfigError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(ImageConfigError::UsageCount { path }),
        Some(_) => Err(ImageConfigError::UsageCount { path }),
    }
}

fn details_image_tokens(
    value: Option<&Json>,
    details_path: &'static str,
    tokens_path: &'static str,
) -> Result<i64, ImageConfigError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Object(details)) => whole_number(details.get("image_tokens"), tokens_path),
        Some(_) => Err(ImageConfigError::UsageDetails { path: details_path }),
    }
}

struct Counts {
    prompt: i64,
    output: i64,
    total: i64,
}

fn usage_counts(usage: &OpenRouterImageUsage) -> Result<Counts, ImageConfigError> {
    let prompt = whole_number(usage.prompt_tokens.as_ref(), "usage.prompt_tokens")?;
    let total = whole_number(usage.total_tokens.as_ref(), "usage.total_tokens")?;
    let output = details_image_tokens(
        usage.completion_tokens_details.as_ref(),
        "usage.completion_tokens_details",
        "usage.completion_tokens_details.image_tokens",
    )?;
    Ok(Counts {
        prompt,
        output,
        total,
    })
}

fn apply_cost(
    usage: &OpenRouterImageUsage,
    hidden: &mut ImageHiddenParams,
) -> Result<(), ImageConfigError> {
    match &usage.cost {
        None | Some(Json::Null) => {}
        Some(Json::Number(number)) => {
            let amount = number
                .as_f64()
                .and_then(Number::from_f64)
                .ok_or(ImageConfigError::Cost)?;
            hidden
                .additional_headers
                .insert(String::from(COST_HEADER), Json::Number(amount));
        }
        Some(_) => return Err(ImageConfigError::Cost),
    }
    match &usage.cost_details {
        None | Some(Json::Null) => Ok(()),
        Some(Json::Object(details)) if details.is_empty() => Ok(()),
        Some(Json::Object(details)) => {
            let merged = hidden
                .response_cost_details
                .get_or_insert_with(IndexMap::new);
            for (name, value) in details {
                merged.insert(name.clone(), value.clone());
            }
            Ok(())
        }
        Some(_) => Err(ImageConfigError::CostDetails),
    }
}

pub fn apply_generation_usage(
    usage: &OpenRouterImageUsage,
    response: &mut ImageResponse,
    hidden: &mut ImageHiddenParams,
) -> Result<(), ImageConfigError> {
    let counts = usage_counts(usage)?;
    response.usage = Some(ImageUsage {
        input_tokens: counts.prompt,
        input_tokens_details: ImageInputTokens {
            image_tokens: 0,
            text_tokens: counts.prompt,
        },
        output_tokens: counts.output,
        total_tokens: counts.total,
    });
    apply_cost(usage, hidden)
}

pub fn apply_edit_usage(
    usage: &OpenRouterImageUsage,
    response: &mut ImageResponse,
    hidden: &mut ImageHiddenParams,
) -> Result<(), ImageConfigError> {
    let counts = usage_counts(usage)?;
    let image_tokens = details_image_tokens(
        usage.prompt_tokens_details.as_ref(),
        "usage.prompt_tokens_details",
        "usage.prompt_tokens_details.image_tokens",
    )?;
    let text_tokens = counts
        .prompt
        .checked_sub(image_tokens)
        .ok_or(ImageConfigError::TextTokens)?;
    response.usage = Some(ImageUsage {
        input_tokens: counts.prompt,
        input_tokens_details: ImageInputTokens {
            image_tokens,
            text_tokens,
        },
        output_tokens: counts.output,
        total_tokens: counts.total,
    });
    apply_cost(usage, hidden)
}
