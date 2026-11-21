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

// the arguments every branch shares, with the few the branch itself decides
struct BranchInput<'call> {
    optional_params: Params,
    api_base: Option<&'call str>,
    api_key: Option<&'call SecretString>,
    client: Option<SuppliedClientKind>,
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
    if plan.slug == "nvidia_riva" {
        return injected(service, InjectedTarget::Riva, None, branch).await;
    }
    if plan.slug == "soniox" {
        return injected(
            service,
            InjectedTarget::Soniox,
            branch.call.extra_headers.as_ref(),
            branch,
        )
        .await;
    }
    if plan.slug == "bedrock" {
        return injected(
            service,
            InjectedTarget::Bedrock,
            branch.call.extra_headers.as_ref(),
            branch,
        )
        .await;
    }
    if plan.config == Some(AudioTranscriptionConfigTag::IBMWatsonX) {
        return watsonx(service, branch).await;
    }
    if plan.config.is_some() {
        return injected(service, InjectedTarget::Native, None, branch).await;
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
        BranchCredentials::Azure(secrets) => azure(service, branch, &secrets).await,
        BranchCredentials::OpenAi(secrets) => openai(service, branch, &secrets).await,
    }
}

async fn azure<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    branch: &BranchCall<'_>,
    secrets: &AzureCredentials,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    let mut optional_params = branch.plan.optional_params.clone();
    // the Azure branch adds extra_headers to the options even when the caller passed none
    optional_params.insert(
        String::from("extra_headers"),
        headers_value(branch.call.extra_headers.as_ref()),
    );
    let input = base_input(
        service.identify,
        branch,
        BranchInput {
            optional_params,
            api_base: secrets.api_base.as_deref(),
            api_key: secrets.api_key.as_ref(),
            client: branch.call.client,
        },
    );
    service
        .dispatch
        .transcribe(TranscriptionTarget::Azure(AzureTranscription {
            input,
            api_version: secrets.api_version.as_deref(),
            azure_ad_token: secrets.azure_ad_token.as_ref(),
        }))
        .await
        .map_err(TranscriptionError::Dispatch)
}

async fn openai<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    branch: &BranchCall<'_>,
    secrets: &OpenAiCredentials,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    let input = base_input(
        service.identify,
        branch,
        BranchInput {
            optional_params: branch.plan.optional_params.clone(),
            api_base: Some(secrets.api_base.as_str()),
            api_key: secrets.api_key.as_ref(),
            client: branch.call.client,
        },
    );
    service
        .dispatch
        .transcribe(TranscriptionTarget::OpenAi(OpenAiTranscription {
            input,
            organization: secrets.organization.as_deref(),
            family: transcription_family(branch.plan.config),
        }))
        .await
        .map_err(TranscriptionError::Dispatch)
}

// Riva, Soniox, Bedrock and the generic native handler: same input, different headers and name
async fn injected<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    operation: InjectedTarget,
    extra_headers: Option<&Headers>,
    branch: &BranchCall<'_>,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    let empty = Headers::new();
    let (client, headers) = match operation {
        InjectedTarget::Riva => (None, None),
        InjectedTarget::Soniox | InjectedTarget::Bedrock => (branch.call.client, extra_headers),
        // the generic handler receives headers={} and keeps only HTTP handler clients
        InjectedTarget::Native => (http_client(branch.call.client), Some(&empty)),
    };
    let target = operation.wrap(InjectedTranscription {
        input: base_input(
            service.identify,
            branch,
            BranchInput {
                optional_params: branch.plan.optional_params.clone(),
                api_base: branch.plan.api_base.as_deref(),
                api_key: branch.plan.api_key.as_ref(),
                client,
            },
        ),
        headers,
        config: branch.plan.config,
    });
    service
        .dispatch
        .transcribe(target)
        .await
        .map_err(TranscriptionError::Dispatch)
}

// IBMWatsonX runs through the dispatch's own watsonx operation, with headers={} and HTTP handler clients
async fn watsonx<Dispatch: TranscriptionDispatch>(
    service: &Transcriptions<'_, Dispatch>,
    branch: &BranchCall<'_>,
) -> Result<TranscriptionAnswer, TranscriptionError<Dispatch::Error>> {
    let empty = Headers::new();
    let target = InjectedTranscription {
        input: base_input(
            service.identify,
            branch,
            BranchInput {
                optional_params: branch.plan.optional_params.clone(),
                api_base: branch.plan.api_base.as_deref(),
                api_key: branch.plan.api_key.as_ref(),
                client: http_client(branch.call.client),
            },
        ),
        headers: Some(&empty),
        config: branch.plan.config,
    };
    service.dispatch.watsonx(target).await
}

// the arguments every branch shares, with the branch's overrides folded in
fn base_input<'call>(
    identify: fn() -> Uuid,
    branch: &BranchCall<'call>,
    overrides: BranchInput<'call>,
) -> TranscriptionInput<'call> {
    TranscriptionInput {
        model: &branch.plan.model,
        input: branch.input,
        file: branch.processed,
        optional_params: overrides.optional_params,
        litellm_params: branch.plan.kwargs.clone(),
        atranscription: matches!(branch.call.atranscription, Some(Json::Bool(true))),
        timeout: branch.timeout,
        max_retries: branch.max_retries,
        api_base: overrides.api_base,
        api_key: overrides.api_key,
        client: overrides.client,
        shared_session: branch.call.shared_session.as_ref(),
        boundary: identify(),
    }
}
