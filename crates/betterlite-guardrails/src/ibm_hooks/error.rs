use thiserror::Error;

use crate::ibm_detector_client::client::error::IbmClientError;

#[derive(Debug, Error)]
pub enum IbmHookError {
    #[error("{message}")]
    Violation { message: String },
    #[error(transparent)]
    Detector(#[from] IbmClientError),
}
