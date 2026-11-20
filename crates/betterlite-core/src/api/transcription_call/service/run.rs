use std::time::Duration;

use betterlite_context::media_config_selection::audio_transcription::AudioTranscriptionConfigTag;
use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::transcription::config::TranscriptionFamily;
use secrecy::SecretString;

use super::Transcriptions;
use crate::api::transcription_call::DEFAULT_TIMEOUT_SECONDS;
use crate::api::transcription_call::call::{SuppliedClientKind, TranscriptionCall};
use crate::api::transcription_call::dispatch::{TranscriptionAnswer, TranscriptionDispatch};
use crate::api::transcription_call::error::TranscriptionError;
use crate::api::transcription_call::plan::{
    BranchCredentials, CredentialScheme, Plan, credentials, prepare,
};
use crate::api::transcription_call::target::{
    AzureTranscription, InjectedTranscription, OpenAiTranscription, TranscriptionInput,
    TranscriptionTarget, retry_count,
};

// what every provider branch reads off the call it is serving
struct BranchCall<'call> {
    call: &'call TranscriptionCall,
    plan: &'call Plan,
    processed: &'call ProcessedAudioFile,
    input: &'call str,
    timeout: Duration,
    max_retries: u32,
}

impl<Dispatch> Transcriptions<'_, Dispatch>
where
    Dispatch: TranscriptionDispatch,
{
    // the sync transcription body: resolve, log, then one provider branch
    pub(super) async fn run(
        &self,
        call: &TranscriptionCall,
        processed: &ProcessedAudioFile,
        input: &str,
    ) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
        let plan = prepare(
            call,
            self.dispatch,
            self.settings,
            &self.resolution,
            self.bundled,
        )?;
        let seconds = call.timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        let timeout = Duration::try_from_secs_f64(seconds)
            .map_err(|_invalid| TranscriptionError::InvalidTimeout { seconds })?;
        let branch = BranchCall {
            call,
            plan: &plan,
            processed,
            input,
            timeout,
            max_retries: retry_count(call.max_retries),
        };
        self.log(call, &plan)?;
        if matches!(plan.slug.as_str(), "azure" | "azure_ai") && plan.config.is_none() {
            return self.provider(CredentialScheme::Azure, &branch).await;
        }
        if plan.openai_transport {
            return self.provider(CredentialScheme::OpenAi, &branch).await;
        }
        if plan.slug == "nvidia_riva" {
            return self.injected(InjectedTarget::Riva, None, &branch).await;
        }
        if plan.slug == "soniox" {
            return self
                .injected(InjectedTarget::Soniox, call.extra_headers.as_ref(), &branch)
                .await;
        }
        if plan.slug == "bedrock" {
            return self
                .injected(
                    InjectedTarget::Bedrock,
                    call.extra_headers.as_ref(),
                    &branch,
                )
                .await;
        }
        if plan.config == Some(AudioTranscriptionConfigTag::IBMWatsonX) {
            let empty = Headers::new();
            let target = InjectedTranscription {
                input: self.base_input(
                    &branch,
                    plan.optional_params.clone(),
                    plan.api_base.as_deref(),
                    plan.api_key.as_ref(),
                    http_client(call.client),
                ),
                headers: Some(&empty),
                config: plan.config,
            };
            return self.dispatch.watsonx(target).await;
        }
        if plan.config.is_some() {
            return self.injected(InjectedTarget::Native, None, &branch).await;
        }
        Err(TranscriptionError::UnmappedProvider)
    }

    // the two transports main selects before its injected branches, each with its own credential chain
    async fn provider(
        &self,
        scheme: CredentialScheme,
        branch: &BranchCall<'_>,
    ) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
        match credentials(scheme, branch.plan, branch.call, self.settings) {
            BranchCredentials::Azure(secrets) => {
                let mut optional_params = branch.plan.optional_params.clone();
                // the Azure branch adds extra_headers to the options even when the caller passed none
                optional_params.insert(
                    String::from("extra_headers"),
                    headers_value(branch.call.extra_headers.as_ref()),
                );
                self.dispatch
                    .transcribe(TranscriptionTarget::Azure(AzureTranscription {
                        input: self.base_input(
                            branch,
                            optional_params,
                            secrets.api_base.as_deref(),
                            secrets.api_key.as_ref(),
                            branch.call.client,
                        ),
                        api_version: secrets.api_version.as_deref(),
                        azure_ad_token: secrets.azure_ad_token.as_ref(),
                    }))
                    .await
                    .map_err(TranscriptionError::Dispatch)
            }
            BranchCredentials::OpenAi(secrets) => self
                .dispatch
                .transcribe(TranscriptionTarget::OpenAi(OpenAiTranscription {
                    input: self.base_input(
                        branch,
                        branch.plan.optional_params.clone(),
                        Some(secrets.api_base.as_str()),
                        secrets.api_key.as_ref(),
                        branch.call.client,
                    ),
                    organization: secrets.organization.as_deref(),
                    family: transcription_family(branch.plan.config),
                }))
                .await
                .map_err(TranscriptionError::Dispatch),
        }
    }

    // Riva, Soniox, Bedrock and the generic native handler: same input, different headers and name
    async fn injected(
        &self,
        target: InjectedTarget,
        extra_headers: Option<&Headers>,
        branch: &BranchCall<'_>,
    ) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
        let empty = Headers::new();
        let (client, headers) = match target {
            InjectedTarget::Riva => (None, None),
            InjectedTarget::Soniox | InjectedTarget::Bedrock => (branch.call.client, extra_headers),
            // the generic handler receives headers={} and keeps only HTTP handler clients
            InjectedTarget::Native => (http_client(branch.call.client), Some(&empty)),
        };
        let operation = InjectedTranscription {
            input: self.base_input(
                branch,
                branch.plan.optional_params.clone(),
                branch.plan.api_base.as_deref(),
                branch.plan.api_key.as_ref(),
                client,
            ),
            headers,
            config: branch.plan.config,
        };
        let target = target.wrap(operation);
        self.dispatch
            .transcribe(target)
            .await
            .map_err(TranscriptionError::Dispatch)
    }

    // the arguments every branch shares; a few branches override the client or the headers
    fn base_input<'call>(
        &self,
        branch: &BranchCall<'call>,
        optional_params: Params,
        api_base: Option<&'call str>,
        api_key: Option<&'call SecretString>,
        client: Option<SuppliedClientKind>,
    ) -> TranscriptionInput<'call> {
        TranscriptionInput {
            model: &branch.plan.model,
            input: branch.input,
            file: branch.processed,
            optional_params,
            litellm_params: branch.plan.kwargs.clone(),
            atranscription: matches!(branch.call.atranscription, Some(Json::Bool(true))),
            timeout: branch.timeout,
            max_retries: branch.max_retries,
            api_base,
            api_key,
            client,
            shared_session: branch.call.shared_session.as_ref(),
            boundary: (self.identify)(),
        }
    }
}

// which injected operation a branch reaches, and the target variant it wraps
#[derive(Clone, Copy, PartialEq, Eq)]
enum InjectedTarget {
    Riva,
    Soniox,
    Bedrock,
    Native,
}

impl InjectedTarget {
    fn wrap(self, operation: InjectedTranscription<'_>) -> TranscriptionTarget<'_> {
        match self {
            Self::Riva => TranscriptionTarget::NvidiaRiva(operation),
            Self::Soniox => TranscriptionTarget::Soniox(operation),
            Self::Bedrock => TranscriptionTarget::Bedrock(operation),
            Self::Native => TranscriptionTarget::Native(operation),
        }
    }
}

// Soniox and the generic native handler keep only the two HTTPHandler classes
fn http_client(client: Option<SuppliedClientKind>) -> Option<SuppliedClientKind> {
    match client {
        Some(SuppliedClientKind::HttpHandler | SuppliedClientKind::AsyncHttpHandler) => client,
        Some(
            SuppliedClientKind::OpenAi
            | SuppliedClientKind::AsyncOpenAi
            | SuppliedClientKind::AzureOpenAi
            | SuppliedClientKind::AsyncAzureOpenAi,
        )
        | None => None,
    }
}

// the OpenAI branch's family is the provider config B157 selected
fn transcription_family(
    config: Option<AudioTranscriptionConfigTag>,
) -> Option<TranscriptionFamily> {
    match config {
        Some(AudioTranscriptionConfigTag::OpenAIWhisper) => Some(TranscriptionFamily::Whisper),
        Some(AudioTranscriptionConfigTag::OpenAIGPT) => Some(TranscriptionFamily::Gpt),
        Some(
            AudioTranscriptionConfigTag::AzureSpeech
            | AudioTranscriptionConfigTag::Deepgram
            | AudioTranscriptionConfigTag::ElevenLabs
            | AudioTranscriptionConfigTag::XAI
            | AudioTranscriptionConfigTag::HostedVLLM
            | AudioTranscriptionConfigTag::IBMWatsonX
            | AudioTranscriptionConfigTag::OVHCloud
            | AudioTranscriptionConfigTag::Scaleway
            | AudioTranscriptionConfigTag::Mistral
            | AudioTranscriptionConfigTag::NvidiaRiva
            | AudioTranscriptionConfigTag::Soniox
            | AudioTranscriptionConfigTag::VertexGemini
            | AudioTranscriptionConfigTag::VertexAI
            | AudioTranscriptionConfigTag::Gemini
            | AudioTranscriptionConfigTag::EdenAI,
        )
        | None => None,
    }
}

// optional_params["extra_headers"] = extra_headers, which keeps a None as null
fn headers_value(headers: Option<&Headers>) -> JsonValue {
    headers.map_or(JsonValue::Null, |headers| {
        JsonValue::Object(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), JsonValue::Text(value.clone())))
                .collect(),
        )
    })
}
