use std::future::Future;
use std::io;
use std::path::Path;

use betterlite_core::api::transcription_call::dispatch::{
    TranscriptionAnswer, TranscriptionDispatch,
};
use betterlite_core::api::transcription_call::error::TranscriptionDispatchError;
use betterlite_core::api::transcription_call::target::TranscriptionTarget;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles;
use thiserror::Error;

use super::{openai, xai};

#[derive(Debug, Error)]
pub(super) enum DispatchFailure {
    // the provider's own status, which both SDKs surface as an API error
    #[error("{message}")]
    Provider {
        status: Option<u16>,
        message: String,
    },
    // a betterlite-only failure the oracle does not compare
    #[error("{message}")]
    Unsupported { message: String },
}

impl TranscriptionDispatchError for DispatchFailure {
    fn status_code(&self) -> Option<u16> {
        match self {
            Self::Provider { status, .. } => *status,
            Self::Unsupported { .. } => None,
        }
    }
}

// The two provider operations this task's cases reach; every other injected target stays unbuilt here
pub(super) struct Dispatch<'client> {
    pub(super) client: &'client ProxiedClient,
}

impl TranscriptionDispatch for Dispatch<'_> {
    type Error = DispatchFailure;

    fn transcribe(
        &self,
        target: TranscriptionTarget<'_>,
    ) -> impl Future<Output = Result<TranscriptionAnswer, Self::Error>> + Send {
        async move {
            match target {
                TranscriptionTarget::OpenAi(operation) => {
                    openai::transcribe(operation, self.client).await
                }
                TranscriptionTarget::Native(operation) => {
                    xai::transcribe(operation, self.client).await
                }
                TranscriptionTarget::Azure(_)
                | TranscriptionTarget::NvidiaRiva(_)
                | TranscriptionTarget::Soniox(_)
                | TranscriptionTarget::Bedrock(_)
                | TranscriptionTarget::Watsonx(_) => Err(DispatchFailure::Unsupported {
                    message: String::from("the oracle serves only the openai and native branches"),
                }),
            }
        }
    }
}

// The case's audio arrives as bytes in a tuple, so no path ever reaches the filesystem
pub(super) struct Files;

impl AuthorizedAudioFiles for Files {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn io::Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }
}
