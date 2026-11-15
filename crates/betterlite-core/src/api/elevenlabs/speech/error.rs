use betterlite_http::error::HttpError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpeechError {
    #[error("the ElevenLabs voice_id is required, pass `voice` when calling speech")]
    VoiceRequired,
    #[error("the ElevenLabs API key is required, set ELEVENLABS_API_KEY")]
    ApiKeyRequired,
    #[error("the voice_id cannot be one URL path segment")]
    VoiceSegment {
        #[source]
        source: HttpError,
    },
    #[error("the speech query parameters must be an object")]
    QueryShape,
    #[error("the speech query parameter {name} must be a string or a number")]
    QueryValue { name: String },
}
