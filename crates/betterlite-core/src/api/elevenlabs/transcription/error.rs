use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranscriptionError {
    #[error("ElevenLabs API key is required. Set ELEVENLABS_API_KEY environment variable.")]
    ApiKeyRequired,
    #[error("the ElevenLabs transcript reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("Error transforming ElevenLabs response: {message}\nResponse: {raw}")]
    Reply { message: String, raw: String },
    #[error("the ElevenLabs form field {field} is not a string, a number or a boolean")]
    FormValue { field: String },
}
