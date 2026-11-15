use crate::content_filter_actions::error::ContentFilterBlockDetail;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_records::error::GuardrailRecordError;

#[derive(Debug, thiserror::Error)]
pub enum StreamFilterError {
    #[error("the content filter blocked the streamed response")]
    Blocked { detail: ContentFilterBlockDetail },
    #[error(transparent)]
    Record(#[from] GuardrailRecordError),
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
}

impl StreamFilterError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Blocked { .. } => Some(400),
            Self::Record(_) | Self::Base(_) => None,
        }
    }
}
