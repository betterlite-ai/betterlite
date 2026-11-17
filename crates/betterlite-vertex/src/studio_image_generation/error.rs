use betterlite_models::image_cost::error::ImageCostError;
use http::header::HeaderMap;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::gemini_image_policy::error::GeminiImagePolicyError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

#[derive(Debug, Error)]
pub enum StudioImageError {
    // validate_environment's bare ValueError, kept word for word
    #[error("GEMINI_API_KEY is not set")]
    MissingApiKey,
    #[error("the Studio image reply is {size} bytes, over the {MAX_RESPONSE_BYTES} byte limit")]
    TooLarge { size: usize },
    // LiteLLM's get_error_class carries the reply's own status and headers for a body it cannot parse
    #[error("the Studio image reply is not JSON")]
    NotJson {
        status: u16,
        headers: Box<HeaderMap>,
        #[source]
        source: JsonError,
    },
    #[error("the Studio image reply does not match the image response at {path}")]
    Shape {
        path: String,
        #[source]
        source: JsonError,
    },
    #[error("the Studio image usageMetadata is not an object")]
    UsageShape,
    #[error("the Studio image token counts in `{field}` overflow")]
    TokenOverflow { field: &'static str },
    #[error("`{param}` must be {expected}")]
    FieldType {
        param: &'static str,
        expected: &'static str,
    },
    // the usage detail maps hold raw JSON, and a decimal integer always encodes; this stays for the signature
    #[error("the Studio image usage details could not be encoded")]
    Encode(#[source] JsonError),
    #[error(transparent)]
    Policy(#[from] GeminiImagePolicyError),
    #[error(transparent)]
    Cost(#[from] ImageCostError),
}

impl StudioImageError {
    // only a body that does not parse keeps the upstream status and headers; every other failure is raised locally
    #[must_use]
    pub fn upstream_answer(&self) -> Option<(u16, &HeaderMap)> {
        match self {
            Self::NotJson {
                status, headers, ..
            } => Some((*status, headers.as_ref())),
            Self::MissingApiKey
            | Self::TooLarge { .. }
            | Self::Shape { .. }
            | Self::UsageShape
            | Self::TokenOverflow { .. }
            | Self::FieldType { .. }
            | Self::Encode(_)
            | Self::Policy(_)
            | Self::Cost(_) => None,
        }
    }
}
