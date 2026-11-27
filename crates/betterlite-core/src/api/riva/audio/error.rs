use betterlite_pyhost::call_error::PythonError;
use pyo3::{PyErr, Python};

/// The cap on a supplied file and on the PCM a decoder hands back, as 64 MiB.
pub const MAX_AUDIO_BYTES: u64 = 67_108_864;

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
    },
    #[error("the audio could not be staged for the decoder: {message}")]
    Staging { message: String },
    #[error("the decoded audio duration is out of range")]
    Duration,
    #[error("the Python worker pool failed: {message}")]
    Work { message: String },
}

impl RivaAudioError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Decode { .. } => 400,
            Self::TooLarge => 413,
            Self::NumpyMissing
            | Self::Python { .. }
            | Self::Staging { .. }
            | Self::Duration
            | Self::Work { .. } => 500,
        }
    }

    #[must_use]
    pub const fn error_type(&self) -> &'static str {
        if self.status_code() < 500 {
            INVALID_REQUEST_ERROR
        } else {
            INTERNAL_SERVER_ERROR
        }
    }

    #[must_use]
    pub const fn param(&self) -> Option<&'static str> {
        match self {
            Self::TooLarge => Some("file"),
            Self::NumpyMissing
            | Self::Decode { .. }
            | Self::Python { .. }
            | Self::Staging { .. }
            | Self::Duration
            | Self::Work { .. } => None,
        }
    }

    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        match self {
            Self::TooLarge => Some(AUDIO_TOO_LARGE),
            Self::NumpyMissing
            | Self::Decode { .. }
            | Self::Python { .. }
            | Self::Staging { .. }
            | Self::Duration
            | Self::Work { .. } => None,
        }
    }
}

pub(super) fn python_error(
    py: Python<'_>,
    operation: &'static str,
    error: &PyErr,
) -> RivaAudioError {
    let failure = PythonError::from_pyerr(py, error);
    RivaAudioError::Python {
        operation,
        type_name: failure.type_name,
        message: failure.message,
    }
}
