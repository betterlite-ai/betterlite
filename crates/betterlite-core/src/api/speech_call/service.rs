use std::time::Duration;

use betterlite_config::sections::speech_call::Settings;
use betterlite_context::media_config_selection::text_to_speech::{
    TextToSpeechConfigTag, select_text_to_speech,
};
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::speech::result::SpeechTransport;
use betterlite_redact::secret::Redactor;
use secrecy::ExposeSecret as _;
use serde_json::Value;

use crate::api::speech_chat_bridge::SpeechCompletions;

use super::call::SpeechCall;
use super::error::{SpeechCallError, SpeechException, exception};
use super::logging::{SpeechCallLogging, record_call};
use super::operations::{
    SpeechConfigOperation, SpeechConfigSource, SpeechMappingOptions, SpeechOperations,
};
use super::providers::openai_compatible;

// the audio reply betterlite reads in full; anything larger is refused instead of buffered
pub(crate) const SPEECH_BODY_LIMIT: usize = TransportLimits::DEFAULT.body_bytes;

// speech() over the services it composes; every provider family arrives through one of the seams
pub struct SpeechDispatch<'service, Transport, Completions, Configs, Operations> {
    pub settings: &'service Settings,
    pub resolution: &'service ResolutionContext<'service>,
    pub transport: &'service Transport,
    pub completions: &'service Completions,
    pub configs: &'service Configs,
    pub operations: &'service Operations,
}

// what every provider branch receives: the call, the resolved model and the already-mapped options
pub(crate) struct BranchCall<'service, 'call> {
    pub(crate) call: &'call SpeechCall,
    pub(crate) resolved: &'call ResolvedModel,
    pub(crate) voice: Option<SpeechVoice>,
    pub(crate) optional_params: Object,
    pub(crate) litellm_params: Object,
    pub(crate) kwargs: Object,
    pub(crate) selected: Option<(
        TextToSpeechConfigTag,
        Option<&'service dyn SpeechConfigOperation>,
    )>,
}

impl<Transport, Completions, Configs, Operations>
    SpeechDispatch<'_, Transport, Completions, Configs, Operations>
where
    Transport: SpeechTransport + Sync,
    Completions: SpeechCompletions<Error = SpeechCallError> + Sync,
    Configs: SpeechConfigSource + Sync,
    Operations: SpeechOperations + Sync,
{
    // speech's body: resolve, map, log, then dispatch to one provider family
    pub async fn speech(
        &self,
        call: &SpeechCall,
        logging: &mut dyn SpeechCallLogging,
        redactor: &Redactor,
    ) -> Result<BinaryContent, SpeechException> {
        let resolved = match self.resolve(call) {
            Ok(resolved) => resolved,
            Err(error) => {
                let provider = call.custom_llm_provider.as_deref().unwrap_or_default();
                return Err(exception(&error, &call.model, provider, redactor));
            }
        };
        let provider = resolved.provider.slug().to_owned();
        match self.dispatch(call, &resolved, logging).await {
            Ok(content) => Ok(content),
            Err(error) => Err(exception(&error, &resolved.model, &provider, redactor)),
        }
    }

    fn resolve(&self, call: &SpeechCall) -> Result<ResolvedModel, SpeechCallError> {
        let request = ResolutionRequest {
            model: &call.model,
            custom_llm_provider: call.custom_llm_provider.as_deref(),
            api_base: call.api_base.as_deref().or(call.base_url.as_deref()),
            api_key: call.api_key.as_ref(),
        };
        resolve(&request, self.resolution).map_err(SpeechCallError::Resolution)
    }

    pub(crate) async fn dispatch(
        &self,
        call: &SpeechCall,
        resolved: &ResolvedModel,
        logging: &mut dyn SpeechCallLogging,
    ) -> Result<BinaryContent, SpeechCallError> {
        // litellm.LlmProviders(custom_llm_provider) rejects every name its enum does not carry
        match &resolved.provider {
            ProviderIdentity::Known(_) => {}
            ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => {
                return Err(SpeechCallError::Sdk {
                    class: "ValueError",
                    message: format!("'{}' is not a valid LlmProviders", resolved.provider.slug()),
                });
            }
        }
        let mut kwargs = call.kwargs.clone();
        kwargs.remove("tags");
        let mut optional_params = call.initial_optional_params()?;
        let mut voice = call.voice.clone();
        let provider = resolved.provider.slug().to_owned();
        let selected = self.select_config(&resolved.model, &resolved.provider);
        if let Some((_, Some(config))) = &selected {
            let options = SpeechMappingOptions {
                voice: voice.clone(),
                drop_params: false,
                kwargs: kwargs.clone(),
            };
            let (mapped_voice, mapped_params) =
                config.map_openai_params(&resolved.model, optional_params, &options);
            voice = mapped_voice;
            optional_params = mapped_params;
        }
        record_call(
            &mut *logging,
            call,
            &resolved.model,
            &provider,
            &optional_params,
            &kwargs,
        )?;
        let litellm_params = self.litellm_params(call, resolved);
        let branch = BranchCall {
            call,
            resolved,
            voice,
            optional_params,
            litellm_params,
            kwargs,
            selected,
        };
        match provider.as_str() {
            "edenai" => self.edenai(branch).await,
            "openai" => self.openai_branch(branch, logging).await,
            "azure" | "azure_ai" => self.azure(branch).await,
            "elevenlabs" => self.elevenlabs(branch).await,
            "vertex_ai" | "vertex_ai_beta" => self.vertex(branch, logging).await,
            "gemini" => self.bridge(branch, logging).await,
            "runwayml" => self.runwayml(branch).await,
            "minimax" => self.minimax(branch).await,
            "mistral" => self.mistral(branch).await,
            "aws_polly" => self.aws_polly(branch).await,
            other if openai_compatible(other) => self.openai_branch(branch, logging).await,
            other => Err(SpeechCallError::Unmapped {
                provider: other.to_owned(),
                message: super::error::unmapped_message(other),
            }),
        }
    }

    // B157 names the config by model and provider; the source supplies the object it names
    fn select_config(
        &self,
        model: &str,
        provider: &ProviderIdentity,
    ) -> Option<(
        TextToSpeechConfigTag,
        Option<&'service dyn SpeechConfigOperation>,
    )> {
        let ProviderIdentity::Known(id) = provider else {
            return None;
        };
        let tag = select_text_to_speech(model, *id, self.configs.is_lyria_model(model))?;
        Some((tag, self.configs.config(tag)))
    }

    // the provider dict get_litellm_params builds, limited to the slots speech and its providers read
    fn litellm_params(&self, call: &SpeechCall, resolved: &ResolvedModel) -> Object {
        let mut params = Object::new();
        let api_key = call.api_key.as_ref().or(resolved.api_key.as_ref());
        params.insert(
            String::from("api_key"),
            api_key.map_or(Value::Null, |key| {
                Value::String(key.expose_secret().to_owned())
            }),
        );
        params.insert(String::from("force_timeout"), Value::from(600_u32));
        params.insert(String::from("custom_llm_provider"), Value::Null);
        params.insert(String::from("api_base"), Value::Null);
        params.insert(
            String::from("metadata"),
            call.metadata.clone().map_or(Value::Null, Value::Object),
        );
        params.insert(String::from("preset_cache_key"), Value::Null);
        params.insert(
            String::from("stream_response"),
            Value::Object(Object::new()),
        );
        for name in ["litellm_call_id", "proxy_server_request", "model_info"] {
            if let Some(value) = call.kwargs.get(name) {
                params.insert(name.to_owned(), value.clone());
            }
        }
        if let Some(provider) = call.kwargs.get("azure_ad_token_provider") {
            params.insert(String::from("azure_ad_token_provider"), provider.clone());
        }
        for name in super::providers::OPTIONAL_KWARGS_KEYS {
            if let Some(value) = call.kwargs.get(*name) {
                params.insert((*name).to_owned(), value.clone());
            }
        }
        params
    }

    pub(crate) fn timeout(&self, call: &SpeechCall) -> Result<Duration, SpeechCallError> {
        let seconds = call.timeout_seconds(self.settings);
        Duration::try_from_secs_f64(seconds).map_err(|_invalid| SpeechCallError::Internal {
            message: format!("timeout must be a non-negative number of seconds, got {seconds}"),
        })
    }

    pub(crate) async fn content(
        &self,
        response: betterlite_openai::speech::result::SpeechResponse,
    ) -> Result<BinaryContent, SpeechCallError> {
        let status = response.status;
        let headers = response.headers.clone();
        let body = response
            .content(SPEECH_BODY_LIMIT)
            .await
            .map_err(SpeechCallError::from)?;
        Ok(BinaryContent {
            status,
            headers,
            body,
        })
    }
}
