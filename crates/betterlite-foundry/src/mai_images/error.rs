use betterlite_openai::openai_base_config::forwarding::OpenAiErrorRecord;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaiImageError {
    #[error("api_base is required for Azure AI MAI image {operation}")]
    ApiBaseRequired { operation: &'static str },
    #[error(
        "Unsupported size value: '{size}'. Use a known size (e.g., '1024x1024') or a custom 'WIDTHxHEIGHT' string."
    )]
    UnsupportedSize { model: String, size: String },
    #[error("Invalid size format: '{size}'. Expected format 'WIDTHxHEIGHT' (e.g., '1024x1024').")]
    InvalidSize { model: String, size: String },
    #[error(
        "Unsupported size value: '{size}'. Azure AI MAI image models require width and height of at least 768 pixels."
    )]
    SizeBelowMinimum { model: String, size: String },
    #[error(
        "Unsupported size value: '{size}'. Azure AI MAI image models accept at most 1056768 total pixels ({width}x{height} is {pixels})."
    )]
    SizeAboveMaximum {
        model: String,
        size: String,
        width: i64,
        height: i64,
        pixels: i128,
    },
    #[error("n={count} is not a whole number of images for model {model}.")]
    CountNotWhole { model: String, count: String },
    #[error(
        "n={count} is not supported for model {model}. The Azure AI MAI image endpoint returns exactly 1 image per request and ignores any count, so a larger value would silently return fewer images than requested. Send one request per image, or set drop_params=True to drop n."
    )]
    CountNotOne { model: String, count: String },
    #[error(
        "Parameter {name} is not supported for model {model}. Supported parameters are n, size and width/height. Set drop_params=True to drop unsupported parameters."
    )]
    UnsupportedParam { model: String, name: String },
    #[error("{}", .record.message)]
    NotJson {
        record: Box<OpenAiErrorRecord>,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Azure AI MAI image reply is not a JSON object")]
    NotAnObject,
    #[error("the Azure AI MAI image field {field} has the wrong type")]
    FieldType { field: &'static str },
    #[error("the Azure AI MAI image field {field} overflows")]
    TokenOverflow { field: &'static str },
}

impl MaiImageError {
    // UnsupportedParamsError is a 400 and the OpenAIError wrapper carries the reply's own status
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedSize { .. }
            | Self::InvalidSize { .. }
            | Self::SizeBelowMinimum { .. }
            | Self::SizeAboveMaximum { .. }
            | Self::CountNotWhole { .. }
            | Self::CountNotOne { .. }
            | Self::UnsupportedParam { .. } => Some(400),
            Self::NotJson { record, .. } => Some(record.status_code),
            Self::NotAnObject | Self::FieldType { .. } | Self::TokenOverflow { .. } => Some(500),
            Self::ApiBaseRequired { .. } => None,
        }
    }

    // every UnsupportedParamsError this module raises names azure_ai as its provider
    #[must_use]
    pub const fn llm_provider(&self) -> Option<&'static str> {
        match self {
            Self::UnsupportedSize { .. }
            | Self::InvalidSize { .. }
            | Self::SizeBelowMinimum { .. }
            | Self::SizeAboveMaximum { .. }
            | Self::CountNotWhole { .. }
            | Self::CountNotOne { .. }
            | Self::UnsupportedParam { .. } => Some("azure_ai"),
            Self::ApiBaseRequired { .. }
            | Self::NotJson { .. }
            | Self::NotAnObject
            | Self::FieldType { .. }
            | Self::TokenOverflow { .. } => None,
        }
    }
}
