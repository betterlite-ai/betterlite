use http::header::HeaderMap;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::gemini_image_policy::error::GeminiImagePolicyError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

#[derive(Debug, Error)]
pub enum GeminiImageError {
    #[error("the Google image reply is {size} bytes, over the {MAX_RESPONSE_BYTES} byte limit")]
    TooLarge { size: usize },
    // LiteLLM's get_error_class carries the reply's own status and headers for a body it cannot parse
    #[error("the Google image reply is not JSON")]
    NotJson {
        status: u16,
        headers: Box<HeaderMap>,
        #[source]
        source: JsonError,
    },
    #[error("the Google image reply does not match the image response at {path}")]
    Shape {
        path: String,
        #[source]
        source: JsonError,
    },
    #[error("`{param}` must be {expected}")]
    FieldType {
        param: &'static str,
        expected: &'static str,
    },
    #[error("the Google image usageMetadata is not an object")]
    UsageShape,
    #[error("the Google image token counts in `{field}` overflow")]
    TokenOverflow { field: &'static str },
    // B1334's tools mapping owns its own error classes, so this one passes them through
    #[error("mapping the Gemini image tools failed")]
    Policy(#[source] GeminiImagePolicyError),
}

impl GeminiImageError {
    // only a body that does not parse keeps the upstream status and headers; every other failure is raised locally
    #[must_use]
    pub fn upstream_answer(&self) -> Option<(u16, &HeaderMap)> {
        match self {
            Self::NotJson {
                status, headers, ..
            } => Some((*status, headers.as_ref())),
            Self::TooLarge { .. }
            | Self::Shape { .. }
            | Self::FieldType { .. }
            | Self::UsageShape
            | Self::TokenOverflow { .. }
            | Self::Policy(_) => None,
        }
    }
}
