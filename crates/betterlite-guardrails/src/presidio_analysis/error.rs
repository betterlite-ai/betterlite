use std::string::FromUtf8Error;

use thiserror::Error;
use tokio::runtime::TryCurrentError;
use tokio::sync::AcquireError;

#[derive(Debug, Error)]
pub enum PresidioAnalysisError {
    #[error("{message}")]
    GuardrailRaised {
        guardrail_name: String,
        message: String,
    },
    #[error("Presidio PII analysis failed: analyzer request")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("Presidio PII analysis failed: analyzer response exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("Presidio PII analysis failed: analyzer response is not JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("Presidio PII analysis failed: analyzer response text is not UTF-8")]
    ResponseEncoding {
        #[source]
        source: FromUtf8Error,
    },
    #[error("Presidio PII analysis failed: no async runtime")]
    NoRuntime {
        #[source]
        source: TryCurrentError,
    },
    #[error("Presidio PII analysis failed: chunk semaphore closed")]
    SemaphoreClosed {
        #[source]
        source: AcquireError,
    },
}

impl PresidioAnalysisError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::GuardrailRaised { .. } => 400,
            Self::Request { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::ResponseEncoding { .. }
            | Self::NoRuntime { .. }
            | Self::SemaphoreClosed { .. } => 500,
        }
    }
}
