use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionRequestData, TranscriptionResponse,
};
use betterlite_openai::openai_base_config::Object;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranscriptionRequest;
pub struct TranscriptionResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranscriptionPrepared;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TranscriptionContractError {
    #[error(
        "AudioTranscriptionConfig needs a request transformation for audio transcription models"
    )]
    MissingRequestTransform,
    #[error(
        "AudioTranscriptionConfig does not need a response transformation for audio transcription models"
    )]
    UnexpectedResponseTransform,
}

// B1001's audio processing produces the filename, bytes and media type a provider transform sends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFile<'audio> {
    pub filename: &'audio str,
    pub content: &'audio [u8],
    pub content_type: &'audio str,
}

pub trait AudioTranscriptionConfig: Send + Sync {
    fn supported_openai_params(&self, model: &str) -> Vec<String>;

    fn transform_audio_request(
        &self,
        _model: &str,
        _audio_file: &AudioFile<'_>,
        _optional_params: &Object,
        _litellm_params: &Object,
    ) -> Result<AudioTranscriptionRequestData, TranscriptionContractError> {
        Err(TranscriptionContractError::MissingRequestTransform)
    }

    fn supports_subtitle_synthesis(&self) -> bool {
        false
    }

    fn has_native_endpoint(&self) -> bool {
        false
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: &Object,
        _litellm_params: &Object,
    ) -> String {
        api_base.unwrap_or_default().to_owned()
    }

    fn transform_audio_response(
        &self,
        _raw_response: &UpstreamResponse,
    ) -> Result<TranscriptionResponse, TranscriptionContractError> {
        Err(TranscriptionContractError::UnexpectedResponseTransform)
    }

    fn provider_specific_params(
        &self,
        model: &str,
        optional_params: &Object,
        _openai_params: &[String],
    ) -> Object {
        let supported = self.supported_openai_params(model);
        let mut kept = Object::new();
        for (name, value) in optional_params {
            // LiteLLM drops null values and the two reserved names before it consults the supported list
            if value.is_null() || name == "model" || name == "OPENAI_TRANSCRIPTION_PARAMS" {
                continue;
            }
            if supported.iter().any(|candidate| candidate == name) {
                continue;
            }
            kept.insert(name.clone(), value.clone());
        }
        kept
    }
}

mod tests;
