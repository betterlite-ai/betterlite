use thiserror::Error;

use crate::guardrail_records::error::GuardrailRecordError;
use crate::purview_dlp_evaluation::content_check::ContentCheckError;
use crate::purview_dlp_evaluation::error_handling::PurviewDlpHttpError;

#[derive(Debug, Error)]
pub enum PurviewHookError {
    #[error(transparent)]
    Http(Box<PurviewDlpHttpError>),
    #[error("the Purview DLP guardrail record could not be written")]
    Record {
        #[source]
        source: Box<GuardrailRecordError>,
    },
}

impl From<PurviewDlpHttpError> for PurviewHookError {
    fn from(error: PurviewDlpHttpError) -> Self {
        Self::Http(Box::new(error))
    }
}

impl From<ContentCheckError> for PurviewHookError {
    fn from(error: ContentCheckError) -> Self {
        match error {
            ContentCheckError::Http(http) => Self::Http(http),
            ContentCheckError::Record { source } => Self::Record { source },
        }
    }
}
