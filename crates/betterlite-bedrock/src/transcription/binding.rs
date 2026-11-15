use std::error::Error as StdError;
use std::fmt::{self, Debug, Formatter};

use async_trait::async_trait;
use betterlite_openai::api::audio_file_input::{AudioFile, AuthorizedAudioFiles};
use betterlite_openai::batches::sdk::Timeout;
use secrecy::SecretString;
use serde::Deserialize;

use super::audio::{AudioPayload, audio_payload};
use super::error::TranscriptionError;
use super::params::Params;

pub struct TranscriptionCall<'call> {
    pub model: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub custom_llm_provider: &'call str,
    pub extra_headers: Option<&'call Params>,
    pub optional_params: &'call Params,
    pub timeout: Option<&'call Timeout>,
}

// optional_params and extra_headers carry AWS secrets, so only their names are printed
impl Debug for TranscriptionCall<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let header_names = self
            .extra_headers
            .map(|headers| headers.keys().collect::<Vec<_>>());
        f.debug_struct("TranscriptionCall")
            .field("model", &self.model)
            .field("api_key", &self.api_key)
            .field("api_base", &self.api_base)
            .field("custom_llm_provider", &self.custom_llm_provider)
            .field("extra_headers", &header_names)
            .field(
                "optional_params",
                &self.optional_params.keys().collect::<Vec<_>>(),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Debug)]
pub struct BindingArguments<'call> {
    pub audio: AudioPayload,
    pub call: &'call TranscriptionCall<'call>,
    pub timeout_seconds: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptionResponse {
    pub text: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum BindingError<E>
where
    E: StdError + 'static,
{
    #[error("the native transcription binding failed")]
    Binding(#[source] E),
    #[error("the binding reply is not a TranscriptionResponse")]
    Response(#[source] serde_json::Error),
}

#[async_trait]
pub trait TranscriptionBinding {
    type Error: StdError + Send + 'static;

    fn transcribe(&self, arguments: &BindingArguments<'_>) -> Result<Params, Self::Error>;

    async fn atranscribe(&self, arguments: &BindingArguments<'_>) -> Result<Params, Self::Error>;
}

pub type Reply<E> = Result<TranscriptionResponse, BindingError<E>>;

pub fn prepare_arguments<'call>(
    audio: AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
    call: &'call TranscriptionCall<'call>,
) -> Result<BindingArguments<'call>, TranscriptionError> {
    let payload = audio_payload(audio, files)?;
    let timeout_seconds = match call.timeout {
        None => None,
        Some(Timeout::Seconds(seconds)) => Some(*seconds),
        Some(Timeout::Phases(phases)) => phases.read,
    };
    Ok(BindingArguments {
        audio: payload,
        call,
        timeout_seconds,
    })
}

fn wrap_response<E>(mapping: &Params) -> Reply<E>
where
    E: StdError + 'static,
{
    serde_json::from_str(&serde_json::to_string(mapping).map_err(BindingError::Response)?)
        .map_err(BindingError::Response)
}

pub fn audio_transcriptions<B>(binding: &B, arguments: &BindingArguments<'_>) -> Reply<B::Error>
where
    B: TranscriptionBinding,
{
    wrap_response(
        &binding
            .transcribe(arguments)
            .map_err(BindingError::Binding)?,
    )
}

// E names the binding's error so the reply carries it without repeating the projection
pub async fn async_audio_transcriptions<B, E>(
    binding: &B,
    arguments: &BindingArguments<'_>,
) -> Reply<E>
where
    B: TranscriptionBinding<Error = E> + Sync,
    E: StdError + Send + 'static,
{
    match binding.atranscribe(arguments).await {
        Ok(mapping) => wrap_response(&mapping),
        Err(error) => Err(BindingError::Binding(error)),
    }
}
