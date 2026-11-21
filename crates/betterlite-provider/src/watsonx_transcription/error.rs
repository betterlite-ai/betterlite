use betterlite_openai::watsonx_audio::error::WatsonxAudioError;
use http::header::ToStrError;
use thiserror::Error;

use crate::api::transcription_http::error::TranscriptionHttpError;

// B5842's audio failures keep their class here; the executor's own failures keep theirs
#[derive(Debug, Error)]
pub enum WatsonxTranscriptionError {
    #[error(transparent)]
    Audio(#[from] WatsonxAudioError),
    #[error(transparent)]
    Http(#[from] TranscriptionHttpError),
    #[error("the {name} header value is not text")]
    Header {
        name: String,
        #[source]
        source: ToStrError,
    },
}
