use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::transcription::error::TranscriptionError;
use crate::transcription::request::{self, TranscriptionData};

#[derive(Debug, Error)]
pub enum HostedVllmAudioTranscriptionError {
    // LiteLLM's own text for this error names rerank, kept so a caller matching on it keeps working
    #[error("api_base must be provided for Hosted VLLM rerank")]
    MissingApiBase,
    #[error(transparent)]
    Data(#[from] TranscriptionError),
}

// get_complete_url takes the api_base the provider resolution already defaulted from HOSTED_VLLM_API_BASE
pub fn get_complete_url(
    api_base: Option<&str>,
) -> Result<String, HostedVllmAudioTranscriptionError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .ok_or(HostedVllmAudioTranscriptionError::MissingApiBase)?
        .trim_end_matches('/');
    if base.ends_with("/v1/audio/transcriptions") {
        return Ok(base.to_owned());
    }
    Ok(format!("{base}/v1/audio/transcriptions"))
}

// transform_audio_transcription_request is the whisper transform without its verbose_json default
pub fn transform_audio_transcription_request(
    model: &str,
    optional_params: &IndexMap<String, Json>,
) -> Result<TranscriptionData, HostedVllmAudioTranscriptionError> {
    request::prepare_data(None, model, optional_params)
        .map_err(HostedVllmAudioTranscriptionError::from)
}
