use std::time::Duration;

use betterlite_context::media_config_selection::audio_transcription::AudioTranscriptionConfigTag;
use betterlite_context::non_default_params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;
use uuid::Uuid;

use super::Transcriptions;
use super::branch::{InjectedTarget, headers_value, http_client, transcription_family};
use super::logging::log;
use crate::api::transcription_call::DEFAULT_TIMEOUT_SECONDS;
use crate::api::transcription_call::call::{SuppliedClientKind, TranscriptionCall};
use crate::api::transcription_call::dispatch::{TranscriptionAnswer, TranscriptionDispatch};
use crate::api::transcription_call::error::TranscriptionError;
use crate::api::transcription_call::plan::{
    AzureCredentials, BranchCredentials, CredentialScheme, OpenAiCredentials, Plan, credentials,
    prepare,
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

// where a branch sends its request, with the credentials that destination replaces
#[derive(Clone, Copy)]
enum Destination<'call> {
    Azure(&'call AzureCredentials),
    OpenAi(&'call OpenAiCredentials),
    Injected {
        operation: InjectedTarget,
        extra_headers: Option<&'call Headers>,
    },
}

impl<'call> Destination<'call> {
    // Soniox and Bedrock carry the caller's extra headers; Riva and the generic handler carry none
    #[must_use]
    const fn injected(operation: InjectedTarget, branch: &BranchCall<'call>) -> Self {
        let extra_headers = match operation {
            InjectedTarget::Soniox | InjectedTarget::Bedrock => branch.call.extra_headers.as_ref(),
            InjectedTarget::Riva | InjectedTarget::Native => None,
        };
        Self::Injected {
            operation,
            extra_headers,
        }
    }

    // the api_base and api_key the destination uses: its own credentials, or the plan's
    fn credentials(self, plan: &'call Plan) -> (Option<&'call str>, Option<&'call SecretString>) {
        match self {
            Self::Azure(secrets) => (secrets.api_base.as_deref(), secrets.api_key.as_ref()),
            Self::OpenAi(secrets) => (Some(secrets.api_base.as_str()), secrets.api_key.as_ref()),
            Self::Injected { .. } => (plan.api_base.as_deref(), plan.api_key.as_ref()),
        }
    }

    // the client the destination passes on: its own, or only HTTP handlers for the generic native one
    const fn client(self, call: &'call TranscriptionCall) -> Option<SuppliedClientKind> {
        match self {
            Self::Azure(_) | Self::OpenAi(_) => call.client,
            Self::Injected { operation, .. } => injected_client(operation, call.client),
        }
    }

    // the plan's optional params, plus the extra_headers only the Azure branch adds
    fn optional_params(self, plan: &'call Plan, call: &'call TranscriptionCall) -> Params {
        let mut params = plan.optional_params.clone();
        if matches!(self, Self::Azure(_)) {
            params.insert(
                String::from("extra_headers"),
                headers_value(call.extra_headers.as_ref()),
            );
        }
        params
    }
}

// the sync transcription body: resolve, log, then one provider branch
pub(super) async fn execute<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    call: &TranscriptionCall,
    processed: &ProcessedAudioFile,
    input: &str,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    let plan = prepare(
        call,
        service.dispatch,
        service.settings,
        &service.resolution,
        service.bundled,
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
    log(service.logging, call, &plan)?;
    dispatch_branch(service, &branch).await
}

// main's branch order: the two transports, then each injected operation, then the generic native one
async fn dispatch_branch<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    branch: &BranchCall<'_>,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    let plan = branch.plan;
    if matches!(plan.slug.as_str(), "azure" | "azure_ai") && plan.config.is_none() {
        return provider(service, CredentialScheme::Azure, branch).await;
    }
    if plan.openai_transport {
        return provider(service, CredentialScheme::OpenAi, branch).await;
    }
    let injected = match plan.slug.as_str() {
        "nvidia_riva" => Some(InjectedTarget::Riva),
        "soniox" => Some(InjectedTarget::Soniox),
        "bedrock" => Some(InjectedTarget::Bedrock),
        _ => None,
    };
    if let Some(operation) = injected {
        return route(service, branch, Destination::injected(operation, branch)).await;
    }
    if plan.config == Some(AudioTranscriptionConfigTag::IBMWatsonX) {
        return watsonx(service, branch).await;
    }
    if plan.config.is_some() {
        let native = Destination::injected(InjectedTarget::Native, branch);
        return route(service, branch, native).await;
    }
    Err(TranscriptionError::UnmappedProvider)
}

// the two transports main selects before its injected branches, each with its own credential chain
async fn provider<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    scheme: CredentialScheme,
    branch: &BranchCall<'_>,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    match credentials(scheme, branch.plan, branch.call, service.settings) {
        BranchCredentials::Azure(secrets) => {
            route(service, branch, Destination::Azure(&secrets)).await
        }
        BranchCredentials::OpenAi(secrets) => {
            route(service, branch, Destination::OpenAi(&secrets)).await
        }
    }
}

// one provider branch: the plan's input with the destination's overrides, then its target
async fn route<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    branch: &BranchCall<'_>,
    destination: Destination<'_>,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    let empty = Headers::new();
    let input = input_for(service.identify, branch, destination);
    let target = match destination {
        Destination::Azure(secrets) => TranscriptionTarget::Azure(AzureTranscription {
            input,
            api_version: secrets.api_version.as_deref(),
            azure_ad_token: secrets.azure_ad_token.as_ref(),
        }),
        Destination::OpenAi(secrets) => TranscriptionTarget::OpenAi(OpenAiTranscription {
            input,
            organization: secrets.organization.as_deref(),
            family: transcription_family(branch.plan.config),
        }),
        Destination::Injected {
            operation,
            extra_headers,
        } => {
            let headers = match operation {
                InjectedTarget::Native => Some(&empty),
                InjectedTarget::Soniox | InjectedTarget::Bedrock => extra_headers,
                InjectedTarget::Riva => None,
            };
            operation.wrap(InjectedTranscription {
                input,
                headers,
                config: branch.plan.config,
            })
        }
    };
    let answer = service.dispatch.transcribe(target).await;
    answer.map_err(TranscriptionError::Dispatch)
}

// the input one destination sends: the plan's values with that destination's three overrides
fn input_for<'call>(
    identify: fn() -> Uuid,
    branch: &BranchCall<'call>,
    destination: Destination<'call>,
) -> TranscriptionInput<'call> {
    let plan = branch.plan;
    let call = branch.call;
    let (api_base, api_key) = destination.credentials(plan);
    TranscriptionInput {
        model: &plan.model,
        input: branch.input,
        file: branch.processed,
        optional_params: destination.optional_params(plan, call),
        litellm_params: plan.kwargs.clone(),
        atranscription: matches!(call.atranscription, Some(Json::Bool(true))),
        timeout: branch.timeout,
        max_retries: branch.max_retries,
        api_base,
        api_key,
        client: destination.client(call),
        shared_session: call.shared_session.as_ref(),
        boundary: identify(),
    }
}

// Riva takes no client; Soniox and Bedrock keep it; the generic native handler keeps only HTTP clients
const fn injected_client(
    operation: InjectedTarget,
    client: Option<SuppliedClientKind>,
) -> Option<SuppliedClientKind> {
    match operation {
        InjectedTarget::Riva => None,
        InjectedTarget::Soniox | InjectedTarget::Bedrock => client,
        InjectedTarget::Native => http_client(client),
    }
}

// IBMWatsonX runs through the dispatch's own watsonx operation, on the generic native input
async fn watsonx<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    branch: &BranchCall<'_>,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    let native = Destination::injected(InjectedTarget::Native, branch);
    let input = input_for(service.identify, branch, native);
    let empty = Headers::new();
    let target = InjectedTranscription {
        input,
        headers: Some(&empty),
        config: branch.plan.config,
    };
    service.dispatch.watsonx(target).await
}
