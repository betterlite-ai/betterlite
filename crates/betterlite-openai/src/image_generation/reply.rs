use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

pub const DEFAULT_SIZE: &str = "1024x1024";
pub const DEFAULT_DALL_E_2_QUALITY: &str = "standard";
pub const DEFAULT_DALL_E_3_QUALITY: &str = "hd";
pub const DEFAULT_GPT_QUALITY: &str = "high";
pub const DEFAULT_OUTPUT_FORMAT: &str = "png";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageDatum {
    pub b64_json: Option<String>,
    pub url: Option<String>,
    pub revised_prompt: Option<String>,
    pub provider_specific_fields: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImageUsageDetails {
    pub image_tokens: i64,
    pub text_tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImageUsage {
    pub input_tokens: i64,
    pub input_tokens_details: ImageUsageDetails,
    pub output_tokens: i64,
    pub total_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageGenerationReply {
    pub created: i64,
    pub data: Vec<ImageDatum>,
    pub usage: ImageUsage,
    pub background: Option<String>,
    pub quality: Option<String>,
    pub output_format: Option<String>,
    pub size: Option<String>,
}

#[derive(Debug, Error)]
pub enum ImageReplyError {
    #[error("the image reply is not JSON")]
    Json(#[from] serde_json::Error),
    #[error("the image reply is not an object")]
    Object,
    #[error("created must be an integer")]
    Created,
    #[error("data must be a list")]
    Data,
    #[error("data[{index}] is not an object")]
    Datum { index: usize },
    #[error("data[{index}].{field} must be a string")]
    DatumText { index: usize, field: &'static str },
    #[error("data[{index}].provider_specific_fields must be an object")]
    DatumFields { index: usize },
    #[error("usage must be an object")]
    Usage,
    #[error("usage.{field} must be an integer")]
    UsageCount { field: &'static str },
    #[error("usage.input_tokens_details must be an object")]
    UsageDetails,
    #[error("usage.input_tokens_details.{field} must be an integer")]
    UsageDetailCount { field: &'static str },
    #[error("{field} must be a string")]
    Text { field: &'static str },
    #[error("{message}")]
    ResponseObject { status: u16, message: String },
}
