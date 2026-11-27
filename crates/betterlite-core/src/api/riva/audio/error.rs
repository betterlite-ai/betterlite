use std::io;

use betterlite_pyhost::blocking_work::WorkError;
use betterlite_pyhost::call_error::PythonError;
use betterlite_pyhost::owned_temp::TempError;
use pyo3::{PyErr, Python};

pub const MAX_AUDIO_BYTES: u64 = 0x0400_0000;

const INVALID_REQUEST_ERROR: &str = "invalid_request_error";
const INTERNAL_SERVER_ERROR: &str = "internal_server_error";
const AUDIO_TOO_LARGE: &str = "audio_too_large";

#[derive(Debug, thiserror::Error)]
pub enum RivaAudioError {
    #[error(
        "numpy is required for Riva audio resampling. Install Riva STT extras to enable automatic audio resampling: `pip install 'litellm[stt-nvidia-riva]'`"
    )]
    NumpyMissing,
    #[error("{message}")]
    Decode { message: String },
    #[error("the audio input exceeds the 64 MiB limit for Riva STT")]
    TooLarge,
    #[error("{operation} failed: {type_name}: {message}")]
    Python {
        operation: &'static str,
        type_name: String,
        message: String,
        #[source]
        source: PythonError,
    },
    #[error("the audio could not be staged for the decoder: {0}")]
    Staging(#[from] TempError),
    #[error("the audio could not be staged for the decoder: {source}")]
    StageWrite {
        #[source]
        source: io::Error,
    },
    #[error(
        "the audio could not be staged for the decoder: the staged audio path {path} is not valid UTF-8"
    )]
    StagedPath { path: String },
    #[error("the decoded audio duration is out of range")]
    Duration,
    #[error("the Python worker pool failed: {0}")]
    Work(#[from] WorkError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RivaAudioErrorResponse {
    pub status_code: u16,
    pub error_type: &'static str,
    pub param: Option<&'static str>,
    pub code: Option<&'static str>,
}

impl RivaAudioError {
    #[must_use]
    pub const fn response(&self) -> RivaAudioErrorResponse {
        match self {
            Self::Decode { .. } => RivaAudioErrorResponse {
                status_code: 400,
                error_type: INVALID_REQUEST_ERROR,
                param: None,
                code: None,
            },
            Self::TooLarge => RivaAudioErrorResponse {
                status_code: 413,
                error_type: INVALID_REQUEST_ERROR,
                param: Some("file"),
                code: Some(AUDIO_TOO_LARGE),
            },
            Self::NumpyMissing
            | Self::Python { .. }
            | Self::Staging(..)
            | Self::StageWrite { .. }
            | Self::StagedPath { .. }
            | Self::Duration
            | Self::Work(..) => RivaAudioErrorResponse {
                status_code: 500,
                error_type: INTERNAL_SERVER_ERROR,
                param: None,
                code: None,
            },
        }
    }
}

pub(super) fn call_failure(
    py: Python<'_>,
    operation: &'static str,
    error: &PyErr,
) -> RivaAudioError {
    let failure = PythonError::from_pyerr(py, error);
    RivaAudioError::Python {
        operation,
        type_name: failure.type_name.clone(),
        message: failure.message.clone(),
        source: failure,
    }
}
