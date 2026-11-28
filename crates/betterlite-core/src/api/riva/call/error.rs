use betterlite_pyhost::blocking_work::WorkError;
use betterlite_pyhost::call_error::PythonError;
use thiserror::Error;

use crate::api::riva::audio::error::RivaAudioError;
use crate::api::riva::records::error::RecordsError;
use crate::api::riva::records::grpc_status::RivaException;
use crate::api::transcription_call::error::TranscriptionDispatchError;

// the source's pinned 400 text for a missing gRPC endpoint, word for word
pub const MISSING_ENDPOINT: &str = "NVIDIA Riva requires `api_base` (host:port for the gRPC endpoint, e.g. `grpc.nvcf.nvidia.com:443` or `localhost:50051`). Set it in litellm_params or via `NVIDIA_RIVA_API_BASE`.";

// the source's 500 text when the optional client is absent
pub const INSTALL_HINT: &str =
    "NVIDIA Riva client is not installed. Install with `pip install 'litellm[stt-nvidia-riva]'`.";

const INVALID_STATUS: u16 = 400;
const INTERNAL_STATUS: u16 = 500;

// Every failure the Riva call can reach, with the boundary it crossed: auth and config keep the
// Python cause, ASR service and stream failures are the source's gRPC-mapped exception, and the
// audio, records and worker failures keep their own types.
#[derive(Debug, Error)]
pub enum RivaCallError {
    #[error("{message}")]
    MissingEndpoint { message: String },
    #[error(transparent)]
    Provider(#[from] RivaException),
    #[error("{message}")]
    Install { message: String },
    #[error("the NVIDIA Riva client could not be imported: {source}")]
    Import {
        #[source]
        source: Box<PythonError>,
    },
    #[error("{operation} failed: {source}")]
    Config {
        operation: &'static str,
        #[source]
        source: Box<PythonError>,
    },
    #[error("the NVIDIA Riva stream returned a value the client does not declare: {message}")]
    Malformed { message: String },
    #[error(transparent)]
    Records(#[from] RecordsError),
    #[error(transparent)]
    Audio(#[from] RivaAudioError),
    #[error(transparent)]
    Work(#[from] WorkError),
    #[error("the Riva request could not be written as JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the optional params are not a JSON object")]
    Params,
    #[error("the resampled audio duration has no floating point form")]
    Duration,
}

impl TranscriptionDispatchError for RivaCallError {
    fn status_code(&self) -> Option<u16> {
        match self {
            // a wrong-typed option names a request field, where the source's unhandled ValueError is a 500
            Self::MissingEndpoint { .. } | Self::Records(_) => Some(INVALID_STATUS),
            Self::Provider(error) => Some(error.status_code),
            // the audio errors carry their own status: 413 for an oversized file, 400 for a decode failure
            Self::Audio(error) => Some(error.response().status_code),
            Self::Install { .. }
            | Self::Import { .. }
            | Self::Config { .. }
            | Self::Malformed { .. }
            | Self::Work(_)
            | Self::Json { .. }
            | Self::Params
            | Self::Duration => Some(INTERNAL_STATUS),
        }
    }
}
