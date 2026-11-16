use thiserror::Error;

#[derive(Debug, Error)]
pub enum NativeSpeechError {
    #[error(
        "api_base is required for Azure AVA TTS. Format: https://{{region}}.api.cognitive.microsoft.com or https://{{region}}.tts.speech.microsoft.com"
    )]
    MissingApiBase,
    #[error("speed {speed} does not fit an Azure SSML rate")]
    SpeedOutOfRange { speed: f64 },
}
