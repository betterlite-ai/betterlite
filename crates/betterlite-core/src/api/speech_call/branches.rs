use betterlite_config::sections::speech_call::Settings;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::speech::result::SpeechTransport;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::api::speech_chat_bridge::SpeechCompletions;

use super::error::SpeechCallError;
use super::logging::SpeechCallLogging;
use super::openai::{first_secret, first_text};
use super::operations::{
    SpeechConfigSource, SpeechOperation, SpeechOperationKind, SpeechOperations,
};
use super::service::{BranchCall, SpeechDispatch};

pub(crate) mod families;

use self::families::{
    VoiceRule, carries_base, first_voice, kind, overwrite_credentials, overwrites_credentials,
    required_config, string_voice, voice_rule,
};

impl<'service, Transport, Completions, Configs, Operations>
    SpeechDispatch<'service, Transport, Completions, Configs, Operations>
where
    Transport: SpeechTransport + Sync,
    Completions: SpeechCompletions<Error = SpeechCallError> + Sync,
    Configs: SpeechConfigSource + Sync,
    Operations: SpeechOperations + Sync,
{
    // the shared skeleton every branch fills in; what differs is the kind, the credentials and the voice
    fn operation(
        &self,
        branch: &BranchCall<'service, '_>,
        kind: SpeechOperationKind,
    ) -> SpeechOperation<'service> {
        SpeechOperation {
            kind,
            provider: branch.resolved.provider.slug().to_owned(),
            config: branch.selected.and_then(|(_, config)| config),
            config_tag: branch.selected.map(|(tag, _)| tag),
            model: branch.resolved.model.clone(),
            input: branch.call.input.clone(),
            voice: branch.voice.clone(),
            optional_params: branch.optional_params.clone(),
            litellm_params: branch.litellm_params.clone(),
            timeout_seconds: branch.call.timeout_seconds(self.settings),
            extra_headers: branch
                .kwargs
                .get("extra_headers")
                .and_then(Value::as_object)
                .cloned(),
            api_base: branch.resolved.api_base.clone(),
            api_key: branch.call.api_key.clone(),
            api_version: branch.call.api_version.clone(),
            organization: branch.call.organization.clone(),
            max_retries: branch.call.max_retries(self.settings),
            azure_ad_token: None,
            azure_ad_token_provider: branch.kwargs.get("azure_ad_token_provider").cloned(),
            client: branch.call.client.clone(),
            aspeech: branch.call.aspeech,
            kwargs: branch.kwargs.clone(),
        }
    }

    // the rules table: the one branch shape the simple provider families share
    pub(crate) async fn generic(
        &self,
        branch: &BranchCall<'service, '_>,
        family: Rules,
    ) -> Result<BinaryContent, SpeechCallError> {
        if let Some(message) = required_config(family)
            && branch.selected.and_then(|(_, config)| config).is_none()
        {
            return Err(SpeechCallError::BadRequest {
                message: message.to_owned(),
            });
        }
        let mut operation = self.operation(branch, kind(family));
        operation.voice = match voice_rule(family) {
            VoiceRule::Keep => operation.voice,
            VoiceRule::String => string_voice(branch.voice.as_ref()),
            VoiceRule::FirstOf(names) => first_voice(branch.voice.as_ref(), names),
        };
        if carries_base(family) {
            operation.litellm_params.insert(
                String::from("api_base"),
                branch
                    .resolved
                    .api_base
                    .clone()
                    .map_or(Value::Null, Value::String),
            );
        }
        if overwrites_credentials(family) {
            overwrite_credentials(&mut operation.litellm_params, branch);
        }
        self.operations.execute(operation).await
    }

    pub(crate) async fn elevenlabs(
        &self,
        branch: &BranchCall<'service, '_>,
    ) -> Result<BinaryContent, SpeechCallError> {
        let Some(SpeechVoice::Name(name)) = &branch.voice else {
            return Err(SpeechCallError::BadRequest {
                message: String::from(
                    "'voice' must resolve to an ElevenLabs voice id for ElevenLabs TTS",
                ),
            });
        };
        let voice = name.trim();
        if voice.is_empty() {
            return Err(SpeechCallError::BadRequest {
                message: String::from(
                    "'voice' must resolve to an ElevenLabs voice id for ElevenLabs TTS",
                ),
            });
        }
        let mut kwargs = branch.kwargs.clone();
        let query = kwargs.remove("__elevenlabs_query_params__");
        let mut litellm_params = branch.litellm_params.clone();
        if let Some(Value::Object(query)) = query {
            litellm_params.insert(
                String::from("__elevenlabs_query_params__"),
                Value::Object(query),
            );
        }
        litellm_params.insert(
            String::from("__elevenlabs_voice_id__"),
            Value::String(voice.to_owned()),
        );
        overwrite_credentials(&mut litellm_params, branch);
        let mut operation = self.operation(branch, SpeechOperationKind::Generic);
        operation.voice = Some(SpeechVoice::Name(voice.to_owned()));
        operation.kwargs = kwargs;
        operation.litellm_params = litellm_params;
        self.operations.execute(operation).await
    }

    pub(crate) async fn azure(
        &self,
        branch: &BranchCall<'service, '_>,
    ) -> Result<BinaryContent, SpeechCallError> {
        if branch.resolved.model.starts_with("speech/") {
            return self.generic(branch, Rules::AzureSpeech).await;
        }
        self.azure_openai(branch).await
    }

    // Azure OpenAI: the voice check, then the Azure credential chains on the shared operation
    async fn azure_openai(
        &self,
        branch: &BranchCall<'service, '_>,
    ) -> Result<BinaryContent, SpeechCallError> {
        let Some(SpeechVoice::Name(_voice)) = &branch.voice else {
            return Err(SpeechCallError::BadRequest {
                message: String::from("'voice' is required to be passed as a string for Azure TTS"),
            });
        };
        let mut operation = self.operation(branch, SpeechOperationKind::AzureOpenAi);
        self.azure_credentials(branch, &mut operation);
        self.operations.execute(operation).await
    }

    // the credential tiers and the azure_ad_token handoff Azure the SDK's config supplies
    fn azure_credentials(
        &self,
        branch: &BranchCall<'service, '_>,
        operation: &mut SpeechOperation<'service>,
    ) {
        let settings = self.settings;
        operation.api_base = first_text([
            branch.resolved.api_base.as_deref(),
            settings.api_base.as_deref(),
            settings.azure_api_base.as_deref(),
            None,
        ])
        .map(str::to_owned);
        operation.api_version = first_text([
            branch.call.api_version.as_deref(),
            settings.api_version.as_deref(),
            settings.azure_api_version.as_deref(),
            None,
        ])
        .map(str::to_owned);
        operation.api_key = first_secret([
            branch.call.api_key.as_ref(),
            settings.api_key.as_ref(),
            settings.azure_key.as_ref(),
            settings.azure_openai_api_key.as_ref(),
            settings.azure_api_key.as_ref(),
        ])
        .cloned();
        operation.azure_ad_token = take_azure_ad_token(&mut operation.optional_params, settings);
        if let Some(headers) = branch
            .kwargs
            .get("extra_headers")
            .and_then(Value::as_object)
            .cloned()
            && !headers.is_empty()
        {
            operation
                .optional_params
                .insert(String::from("extra_headers"), Value::Object(headers));
        }
    }

    pub(crate) async fn vertex(
        &self,
        branch: &BranchCall<'service, '_>,
        logging: &mut dyn SpeechCallLogging,
    ) -> Result<BinaryContent, SpeechCallError> {
        if branch.resolved.model.contains("gemini") {
            return self.bridge(branch, logging).await;
        }
        let mut operation = self.operation(branch, SpeechOperationKind::Dispatch);
        operation.extra_headers = branch.call.headers.clone();
        operation.api_key = None;
        operation.api_base = branch.resolved.api_base.clone();
        for name in ["vertex_project", "vertex_location", "vertex_credentials"] {
            let value = branch.kwargs.get(name).cloned().unwrap_or(Value::Null);
            operation.litellm_params.insert(name.to_owned(), value);
        }
        self.operations.execute(operation).await
    }
}

// the options extra_body token wins over AZURE_AD_TOKEN, and an empty one falls back
pub(super) fn take_azure_ad_token(
    optional_params: &mut Object,
    settings: &Settings,
) -> Option<SecretString> {
    let popped = optional_params
        .get_mut("extra_body")
        .and_then(Value::as_object_mut)
        .and_then(|body| body.remove("azure_ad_token"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|value| !value.is_empty());
    popped.map(SecretString::from).or_else(|| {
        settings
            .azure_ad_token
            .clone()
            .filter(|token| !token.expose_secret().is_empty())
    })
}
