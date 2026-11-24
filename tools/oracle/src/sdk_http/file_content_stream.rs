use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_content_stream::target::{CallerClient, SuppliedSdk};
use betterlite_core::api::file_content_stream::{
    ContentAnswer, ContentCall, ContentContext, ContentEntry, ContentKwargs, FileContentStream,
};
use betterlite_core::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
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
use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod fixtures;
pub mod outcome;
mod wire;

// the header a supplied HTTPHandler's own client adds, so the wire shows whether the handler was used
const HANDLER_HEADER: (&str, &str) = ("x-supplied-client", "http-handler");
const DEFAULT_CHUNK_SIZE: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    FileContent,
    AfileContent,
}

// the client kwarg: none, an OpenAI or Azure SDK client, or a LiteLLM HTTPHandler
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuppliedKind {
    #[default]
    None,
    OpenaiSdk,
    AzureSdk,
    HttpHandler,
}

// one SDK call; api_base is always the mock's, and every other kwarg is passed only when the step names it
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub function: Function,
    pub file_id: String,
    #[serde(default)]
    pub model: Option<String>,
    // the Python step declares the provider as text, so an unnamed one is omitted rather than sent as null
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub timeout: Option<ReadTimeout>,
    #[serde(default)]
    pub request_timeout: Option<ReadTimeoutValue>,
    #[serde(default)]
    pub max_retries: Option<i64>,
    // an absent size is LiteLLM's 1048576 default
    #[serde(default)]
    pub chunk_size: Option<u64>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub client: SuppliedKind,
    // the case hands over a real logging object, so the identity file_content_streaming writes is observable
    #[serde(default)]
    pub supplied_logging: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub steps: Vec<StepOutcome>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.steps.is_empty() || self.steps.len() > limits.max_requests {
            return Err(SdkHttpError::Invalid(format!(
                "a file content call needs 1 to {} steps and has {}",
                limits.max_requests,
                self.steps.len()
            )));
        }
        Ok(())
    }

    async fn send(
        &self,
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
        let client = ProxiedClient::outbound(&policy)
            .map_err(|error| SdkHttpError::Client(Box::new(error)))?;
        let catalog = ModelCatalog::default();
        let names = BTreeSet::new();
        let resolution = ResolutionContext {
            catalog: &catalog,
            known_model_names: &names,
            custom_providers: &names,
            environment: env,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
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
        let mut steps = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            steps.push(run_step(&content, step, api_base, &handler).await?);
        }
        Ok(SdkResult::FileContentStream {
            value: Box::new(Outcome { steps }),
        })
    }
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
    let context = ContentContext {
        trusted: None,
        client: caller(step, handler),
        logger: None,
        logging: logging.clone(),
    };
    let answer = content.run(entry, &call, context).await;
    let recorded = logging
        .as_ref()
        .map(|logging| logging.identities())
        .unwrap_or_default();
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
fn caller<'call>(step: &Step, handler: &'call Handler) -> Option<CallerClient<'call, Handler>> {
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
