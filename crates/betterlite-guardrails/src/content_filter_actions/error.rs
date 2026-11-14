use reqwest::StatusCode;
use serde::Serialize;
use thiserror::Error;

use crate::content_filter_matching::ContentFilterMatchingError;
use crate::pattern_matching::PatternError;
use crate::pattern_matching::masking::MaskError;
use crate::primitives::pattern_matching::error::PatternError as ReplacementError;

// The HTTP detail LiteLLM raises with status 400, one variant per match that blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ContentFilterBlockDetail {
    Conditional {
        error: String,
        category: String,
        matched_phrase: String,
        severity: String,
    },
    CategoryKeyword {
        error: String,
        category: String,
        keyword: String,
        severity: String,
    },
    Pattern {
        error: String,
        pattern: String,
    },
    BlockedWord {
        error: String,
        keyword: String,
        description: Option<String>,
    },
}

#[derive(Debug, Error)]
pub enum ContentFilterActionError {
    #[error("the content filter blocked the text")]
    Blocked { detail: ContentFilterBlockDetail },
    #[error(transparent)]
    Matching(#[from] ContentFilterMatchingError),
    #[error(transparent)]
    Redaction(#[from] MaskError),
    #[error(transparent)]
    Pattern(#[from] PatternError),
    #[error(transparent)]
    Replacement(#[from] ReplacementError),
    #[error(transparent)]
    Keyword(#[from] regex::Error),
}

impl ContentFilterActionError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Blocked { .. } => StatusCode::BAD_REQUEST,
            Self::Matching(_)
            | Self::Redaction(_)
            | Self::Pattern(_)
            | Self::Replacement(_)
            | Self::Keyword(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
