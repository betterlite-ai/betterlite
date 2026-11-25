use std::sync::Arc;

use betterlite_core::api::file_content_stream::target::{CallerClient, SuppliedSdk};
use betterlite_core::api::file_content_stream::{
    ContentAnswer, ContentCall, ContentContext, ContentEntry, ContentKwargs, FileContentStream,
};
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_models::controls::call_params::TrustedCredentials;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use self::fixtures::logging::RecordingLogging;
use self::fixtures::setup::send_steps;
use self::outcome::{StepOutcome, identity};
use self::wire::{ContentDispatch, Handler};
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

#[path = "file_content_stream/fixtures/cases.rs"]
mod cases;
#[path = "file_content_stream/fixtures/clients.rs"]
mod clients;
mod fixtures;
#[path = "file_content_stream/fixtures/outcome.rs"]
pub mod outcome;
#[path = "file_content_stream/fixtures/wire.rs"]
mod wire;

use self::cases::{Function, Step, SuppliedKind};

const DEFAULT_CHUNK_SIZE: u64 = 1024 * 1024;

// the case schema's call; SdkHttpCall holds one, so it keeps the public path calls.rs names
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub steps: Vec<StepOutcome>,
}

async fn run_steps(
    content: &FileContentStream<'_, ContentDispatch>,
    call: &Call,
    api_base: &str,
    handler: &Handler,
) -> Result<SdkResult, SdkHttpError> {
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        let trusted = TrustedCredentials {
            values: step
                .azure_ad_token
                .iter()
                .map(|token| {
                    (
                        String::from("azure_ad_token"),
                        SecretString::from(token.clone()),
                    )
                })
                .collect(),
        };
        steps.push(run_step(content, step, api_base, handler, &trusted).await?);
    }
    Ok(SdkResult::FileContentStream {
        value: Box::new(Outcome { steps }),
    })
}

fn content_kwargs(step: &Step, api_base: &str) -> ContentKwargs {
    ContentKwargs {
        api_key: step.api_key.clone().map(SecretString::from),
        api_base: Some(api_base.to_owned()),
        api_version: step.api_version.clone(),
        organization: step.organization.clone(),
        timeout: step.timeout.clone(),
        request_timeout: step.request_timeout.clone(),
        max_retries: step.max_retries,
        litellm_call_id: None,
        id: None,
    }
}

async fn run_step(
    content: &FileContentStream<'_, ContentDispatch>,
    step: &Step,
    api_base: &str,
    handler: &Handler,
    trusted: &TrustedCredentials,
) -> Result<StepOutcome, SdkHttpError> {
    let kwargs = content_kwargs(step, api_base);
    let call = ContentCall {
        file_id: &step.file_id,
        model: step.model.as_deref(),
        custom_llm_provider: step.custom_llm_provider.clone(),
        extra_headers: step.extra_headers.clone(),
        extra_body: step.extra_body.clone(),
        chunk_size: step.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
        stream: step.stream,
        kwargs: &kwargs,
    };
    let entry = match step.function {
        Function::FileContent => ContentEntry::Sync,
        Function::AfileContent => ContentEntry::Async,
    };
    let logging = step
        .supplied_logging
        .then(|| Arc::new(RecordingLogging::default()));
    let invocation = ContentContext {
        trusted: Some(trusted),
        client: caller(step, handler),
        logger: None,
        logging: logging.clone(),
    };
    let answer = content.run(entry, &call, invocation).await;
    let recorded = logging
        .as_deref()
        .map_or_else(Vec::new, RecordingLogging::identities);
    match answer {
        Ok(ContentAnswer::Buffered(value)) => {
            Ok(StepOutcome::buffered(*value, identity(&recorded)))
        }
        Ok(ContentAnswer::Streamed(value)) => {
            StepOutcome::streamed(*value, identity(&recorded)).await
        }
        Err(error) => StepOutcome::failed(&error, None),
    }
}

// The handler's own mode decides whether the provider can use it
const fn caller<'call>(
    step: &Step,
    handler: &'call Handler,
) -> Option<CallerClient<'call, Handler>> {
    match step.client {
        SuppliedKind::None => None,
        SuppliedKind::OpenaiSdk => Some(CallerClient::Sdk(SuppliedSdk::OpenAi)),
        SuppliedKind::AzureSdk => Some(CallerClient::Sdk(SuppliedSdk::Azure)),
        SuppliedKind::HttpHandler => Some(CallerClient::Http(SuppliedClient {
            kind: ClientKind::Sync,
            transport: handler,
        })),
    }
}
