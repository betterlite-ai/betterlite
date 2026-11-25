use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_content_stream::target::{CallerClient, SuppliedSdk};
use betterlite_core::api::file_content_stream::{
    ContentAnswer, ContentCall, ContentContext, ContentEntry, ContentKwargs, FileContentStream,
};
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::FilesApi;
use chrono::Utc;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use self::fixtures::anthropic_settings;
use self::fixtures::logging::RecordingLogging;
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

// the header a supplied HTTPHandler's own client adds, so the wire shows whether the handler was used
const HANDLER_HEADER: (&str, &str) = ("x-supplied-client", "http-handler");
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

async fn send_steps(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let resolution = model_resolution(&catalog, &names, env);
    let content = FileContentStream {
        dispatch: ContentDispatch {
            openai: FilesApi {
                transport: client.clone(),
                boundaries: Uuid::new_v4,
            },
            client,
            anthropic: anthropic_settings(&config.sections.anthropic_credentials),
            environment: env.clone(),
        },
        settings: config.sections.resource_sdk_settings,
        resolution: &resolution,
        clock: || Utc::now().fixed_offset(),
        call_ids: Uuid::new_v4,
    };
    let handler = Handler {
        headers: BTreeMap::from([(HANDLER_HEADER.0.to_owned(), HANDLER_HEADER.1.to_owned())]),
    };
    run_steps(&content, call, api_base, &handler).await
}

// the catalog and environment the resolver reads, held by send_steps for the call's lifetime
fn model_resolution<'deps>(
    catalog: &'deps ModelCatalog,
    names: &'deps BTreeSet<String>,
    env: &'deps BTreeMap<String, String>,
) -> ResolutionContext<'deps> {
    ResolutionContext {
        catalog,
        known_model_names: names,
        custom_providers: names,
        environment: env,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

async fn run_steps(
    content: &FileContentStream<'_, ContentDispatch>,
    call: &Call,
    api_base: &str,
    handler: &Handler,
) -> Result<SdkResult, SdkHttpError> {
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        steps.push(run_step(content, step, api_base, handler).await?);
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
        trusted: None,
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

// no case hands over a trusted snapshot, and the handler's own mode decides whether it is used
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
