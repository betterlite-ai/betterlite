use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_provider_read::target::{FileListing, ReadAnswer, ReadOperation};
use betterlite_core::api::file_provider_read::{
    CallerClient, FileProviderRead, Kwarg, ReadContext, ReadEntry, ReadKwargs, ReadProvider,
};
use betterlite_core::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::files::FilesApi;
use chrono::Utc;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use self::outcome::StepOutcome;
use self::wire::{Handler, HttpDispatch};
use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod fixtures;
pub mod outcome;
mod wire;

// the header a supplied HTTPHandler's own client adds, so the wire shows whether the handler was used
const HANDLER_HEADER: (&str, &str) = ("x-supplied-client", "http-handler");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    FileRetrieve,
    AfileRetrieve,
    FileList,
    AfileList,
}

// the client kwarg: none, an OpenAI SDK client aimed at another path, or a LiteLLM HTTPHandler
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuppliedKind {
    #[default]
    None,
    OpenaiSdk,
    HttpHandler,
}

// one SDK call; api_base is always the mock's, and every other kwarg is passed only when the step names it
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub function: Function,
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
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
    #[serde(default)]
    pub is_async: bool,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub client: SuppliedKind,
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
                "a file read call needs 1 to {} steps and has {}",
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
        let read = FileProviderRead {
            dispatch: HttpDispatch {
                openai: FilesApi {
                    transport: client.clone(),
                    boundaries: Uuid::new_v4,
                },
                client,
                credentials: config.sections.anthropic_credentials,
                environment: env.clone(),
            },
            settings: config.sections.resource_sdk_settings,
            clock: || Utc::now().fixed_offset(),
            call_ids: Uuid::new_v4,
        };
        let handler = Handler {
            headers: BTreeMap::from([(HANDLER_HEADER.0.to_owned(), HANDLER_HEADER.1.to_owned())]),
        };
        let mut steps = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            steps.push(run_step(&read, step, api_base, &handler).await?);
        }
        Ok(SdkResult::FileProviderRead {
            value: Box::new(Outcome { steps }),
        })
    }
}

fn kwargs(step: &Step, api_base: &str) -> ReadKwargs {
    ReadKwargs {
        custom_llm_provider: step
            .custom_llm_provider
            .clone()
            .map_or_else(ReadProvider::default, ReadProvider),
        extra_headers: step.extra_headers.clone(),
        extra_body: step.extra_body.clone(),
        api_base: Some(api_base.to_owned()),
        api_key: step.api_key.clone().map(SecretString::from),
        api_version: step.api_version.clone(),
        organization: step.organization.clone(),
        timeout: step.timeout.clone(),
        request_timeout: step.request_timeout.clone(),
        max_retries: step.max_retries,
        is_async: step.is_async,
        litellm_call_id: Kwarg::Absent,
        id: Kwarg::Absent,
        after: step.after.clone(),
        limit: step.limit,
        order: step.order.clone(),
    }
}

async fn run_step(
    read: &FileProviderRead<HttpDispatch>,
    step: &Step,
    api_base: &str,
    handler: &Handler,
) -> Result<StepOutcome, SdkHttpError> {
    let call = kwargs(step, api_base);
    let mode = match step.function {
        Function::FileRetrieve | Function::FileList => ReadEntry::Sync,
        Function::AfileRetrieve | Function::AfileList => ReadEntry::Async,
    };
    let operation = match step.function {
        Function::FileRetrieve | Function::AfileRetrieve => ReadOperation::Retrieve {
            file_id: step.file_id.as_str(),
        },
        Function::FileList | Function::AfileList => ReadOperation::List {
            purpose: step.purpose.as_deref(),
        },
    };
    let supplied = context(step, handler);
    let answer = match read.run(mode, operation, &call, supplied).await {
        Ok(answer) => answer,
        Err(error) => return StepOutcome::failed(&error),
    };
    // the branch's own list shape comes back as it is: a page from the SDK, a bare list from a provider config
    let (file, listing) = match answer {
        ReadAnswer::File(file) => (Some(serde_json::to_value(file)), None),
        ReadAnswer::Listing(FileListing::Page(page)) => (None, Some(serde_json::to_value(page))),
        ReadAnswer::Listing(FileListing::Files(files)) => (None, Some(serde_json::to_value(files))),
    };
    Ok(StepOutcome {
        file: file.transpose().map_err(SdkHttpError::Serialize)?,
        listing: listing.transpose().map_err(SdkHttpError::Serialize)?,
        failure: None,
        placeholder: None,
    })
}

// no case hands over a trusted snapshot or a logging object, since only server code has them
const fn context<'call>(step: &Step, handler: &'call Handler) -> ReadContext<'call, Handler> {
    ReadContext {
        trusted: None,
        client: match step.client {
            SuppliedKind::None => None,
            SuppliedKind::OpenaiSdk => Some(CallerClient::Sdk),
            SuppliedKind::HttpHandler => Some(CallerClient::Http(SuppliedClient {
                kind: ClientKind::Sync,
                transport: handler,
            })),
        },
        logger: None,
    }
}
