mod config;
pub mod error;
mod source;

mod tests;

use std::time::Duration;

use betterlite_core::api::vector_store_http_write::service::{LogContext, SuppliedClient};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::client::SharedSession;
use betterlite_openai::watsonx_audio::WatsonxAudioConfig;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};

use self::config::{BoundConfig, header_index};
use self::error::WatsonxTranscriptionError;
use self::source::PooledSource;
use crate::api::transcription_contract::AudioFile;
use crate::api::transcription_http::reply::TranscriptionReply;
use crate::api::transcription_http::{TranscriptionCall, TranscriptionDeps, audio_transcriptions};

// The provider tag B5360's selector answers, named in the executor's errors as llm_http_handler.py does
const WATSONX: &str = "watsonx";

// The arguments main.py's transcription branch hands the injected Watsonx operation
pub struct WatsonxTranscriptionCall<'call> {
    pub model: &'call str,
    pub audio_file: AudioFile<'call>,
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub optional_params: &'call Object,
    pub litellm_params: &'call Object,
    pub timeout: Option<Duration>,
    pub asynchronous: bool,
    pub supplied: Option<SuppliedClient<'call, ProxiedClient>>,
    pub shared_session: Option<&'call SharedSession>,
}

// B8331 hands in the resolved environment and the production IAM transport inside the auth service
pub struct WatsonxTranscription<'deps, 'log, 'env, T> {
    pub auth: &'deps WatsonxAuth<T>,
    pub client: &'deps ProxiedClient,
    pub log: &'deps mut LogContext<'log, 'env>,
}

impl<T: WatsonxTransport> WatsonxTranscription<'_, '_, '_, T> {
    pub async fn transcribe(
        &mut self,
        call: &WatsonxTranscriptionCall<'_>,
    ) -> Result<TranscriptionReply, WatsonxTranscriptionError> {
        let headers = WatsonxAudioConfig { auth: self.auth }
            .validate_environment(HeaderMap::new(), call.optional_params, call.api_key)
            .await?;
        let config = BoundConfig::new(self.auth, header_index(&headers)?);
        let source = PooledSource {
            client: self.client.clone(),
        };
        let deps = TranscriptionDeps {
            config: Some(&config),
            source: &source,
            supplied: compatible(call.supplied.as_ref()),
        };
        let outcome = audio_transcriptions(&deps, &forward(call), &mut *self.log).await;
        if let Some(failure) = config.take_failure() {
            return Err(WatsonxTranscriptionError::Audio(failure));
        }
        outcome.map_err(WatsonxTranscriptionError::Http)
    }
}

// A supplied client of the other kind stays unused, as llm_http_handler.py's isinstance check leaves it
fn compatible<'call>(
    supplied: Option<&SuppliedClient<'call, ProxiedClient>>,
) -> Option<SuppliedClient<'call, ProxiedClient>> {
    supplied.map(|client| SuppliedClient {
        kind: client.kind,
        transport: client.transport,
    })
}

fn forward<'call>(call: &WatsonxTranscriptionCall<'call>) -> TranscriptionCall<'call> {
    TranscriptionCall {
        model: call.model,
        audio_file: call.audio_file,
        optional_params: call.optional_params,
        litellm_params: call.litellm_params,
        api_key: call.api_key.map(|key| key.expose_secret()),
        api_base: call.api_base,
        custom_llm_provider: WATSONX,
        headers: None,
        atranscription: Json::Bool(call.asynchronous),
        timeout: call.timeout,
        max_retries: 0_u32,
        shared_session: call.shared_session,
    }
}
