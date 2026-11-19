use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::client::SharedSession;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::transcription_contract::AudioTranscriptionConfig;

use super::wire::TranscriptionTransport;

pub type Headers = IndexMap<String, String>;

// A provider config that refuses the call raises this; llm_http_handler.py lets it escape without _handle_error
#[derive(Debug, Error)]
#[error("{message}")]
pub struct ConfigFailure {
    pub message: String,
}

// The arguments BaseAudioTranscriptionConfig.validate_environment is handed, in LiteLLM's order
pub struct Validation<'call> {
    pub api_key: Option<&'call str>,
    pub headers: &'call Headers,
    pub model: &'call str,
    pub messages: &'call [Object],
    pub optional_params: &'call Object,
    pub litellm_params: &'call Object,
}

// The arguments BaseAudioTranscriptionConfig.get_complete_url is handed
pub struct UrlInput<'call> {
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call str>,
    pub model: &'call str,
    pub optional_params: &'call Object,
    pub litellm_params: &'call Object,
}

// What get_async_httpx_client reads: params={"ssl_verify": ...} and the caller's shared session
pub struct ClientRequest<'call> {
    pub ssl_verify: Option<bool>,
    pub shared_session: Option<&'call SharedSession>,
}

pub trait ClientSource: Send + Sync {
    type Client: TranscriptionTransport;

    fn client(&self, request: ClientRequest<'_>) -> Result<Self::Client, ClientError>;
}

pub trait TranscriptionHttpConfig: AudioTranscriptionConfig {
    fn validate_environment(&self, input: &Validation<'_>) -> Result<Headers, ConfigFailure>;

    // The base config omits api_key from complete_url; an override that needs it reads the input
    fn complete_transcription_url(&self, input: &UrlInput<'_>) -> Result<String, ConfigFailure> {
        Ok(self.complete_url(
            input.api_base,
            input.model,
            input.optional_params,
            input.litellm_params,
        ))
    }
}
