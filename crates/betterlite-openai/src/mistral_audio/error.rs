use base64::DecodeError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MistralAudioError {
    #[error("no Mistral API key is set; pass api_key or set MISTRAL_API_KEY")]
    MissingApiKey,
    #[error("the Mistral reply is not JSON")]
    ReplyNotJson { status: u16 },
    #[error("the Mistral reply is not a JSON object")]
    ReplyNotObject,
    #[error("the Mistral speech reply carries no audio_data")]
    AudioDataMissing,
    #[error("the Mistral speech reply's audio_data is not base64")]
    AudioNotBase64 {
        #[source]
        source: DecodeError,
    },
    #[error("the Mistral reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the Mistral transcription reply's text is not a string")]
    TextNotString,
    #[error("the Mistral transcription field `{field}` cannot be sent as a form value")]
    FormValue { field: String },
}

impl MistralAudioError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::MissingApiKey => 401,
            Self::FormValue { .. } => 400,
            Self::ReplyNotJson { status } => *status,
            Self::ReplyNotObject
            | Self::AudioDataMissing
            | Self::AudioNotBase64 { .. }
            | Self::ReplyTooLarge { .. }
            | Self::TextNotString => 500,
        }
    }
}
