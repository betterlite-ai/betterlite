use http::header::HeaderMap;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

#[derive(Debug, Error)]
pub enum ImagenGenerationError {
    #[error("the Imagen reply is {size} bytes, over the {MAX_RESPONSE_BYTES} byte limit")]
    TooLarge { size: usize },
    // LiteLLM's image error class carries the reply's own status and headers for a body it cannot parse
    #[error("the Imagen reply is not JSON")]
    NotJson {
        status: u16,
        headers: Box<HeaderMap>,
        #[source]
        source: JsonError,
    },
    #[error("the Imagen reply does not match the prediction response at {path}")]
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
}

impl ImagenGenerationError {
    // only a body that does not parse keeps the upstream status and headers; every other failure is raised locally
    #[must_use]
    pub fn upstream_answer(&self) -> Option<(u16, &HeaderMap)> {
        match self {
            Self::NotJson {
                status, headers, ..
            } => Some((*status, headers.as_ref())),
            Self::TooLarge { .. } | Self::Shape { .. } | Self::FieldType { .. } => None,
        }
    }
}
