use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_anthropic::files::api::FilesApi as AnthropicApi;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::assistant_threads::error::sdk_class;
use betterlite_core::api::file_provider_read::error::{
    FileReadError, UNSUPPORTED_BODY, UNSUPPORTED_MODEL, UNSUPPORTED_URL,
};
use betterlite_core::api::file_provider_read::target::FileListing;
use betterlite_core::api::file_provider_read::{
    CallerClient, FileProviderRead, Kwarg, ReadContext, ReadEntry, ReadKwargs, ReadProvider,
};
use betterlite_core::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::files::FilesApi;
use betterlite_openai::files::error::{FilesError, StatusError};
use chrono::Utc;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use self::wire::{Handler, HttpDispatch, ReadWireError};
use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

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

// the placeholder response LiteLLM attaches to its unsupported-provider 400
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placeholder {
    pub body: String,
    pub method: String,
    pub url: String,
    pub model: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Placeholder>,
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
                anthropic: AnthropicApi {
                    transport: client,
                    clock: || Utc::now().timestamp(),
                },
                credentials: config.sections.anthropic_credentials,
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
    let supplied = context(step, handler);
    let purpose = step.purpose.as_deref();
    // the branch's own list shape comes back as it is: a page from the SDK, a bare list from a provider config
    let (file, listing) = match step.function {
        Function::FileRetrieve | Function::AfileRetrieve => {
            match read.retrieve(mode, &step.file_id, &call, supplied).await {
                Ok(file) => (Some(serde_json::to_value(file)), None),
                Err(error) => return failure(&error),
            }
        }
        Function::FileList | Function::AfileList => {
            match read.list(mode, purpose, &call, supplied).await {
                Ok(FileListing::Page(page)) => (None, Some(serde_json::to_value(page))),
                Ok(FileListing::Files(files)) => (None, Some(serde_json::to_value(files))),
                Err(error) => return failure(&error),
            }
        }
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

// LiteLLM's own 400 and its ValueErrors are what both sides raise; a provider status error keeps the SDK's fields
fn failure(error: &FileReadError<ReadWireError>) -> Result<StepOutcome, SdkHttpError> {
    let (failure, placeholder) = match error {
        FileReadError::Unsupported { provider, function } => (
            Failure {
                class: "BadRequestError".to_owned(),
                status: 400,
                kind: Some(None),
                param: Some(None),
                code: Some(None),
                message: format!("litellm.BadRequestError: {error}"),
            },
            Some(Placeholder {
                body: UNSUPPORTED_BODY.to_owned(),
                method: function.request_method().to_owned(),
                url: UNSUPPORTED_URL.to_owned(),
                model: UNSUPPORTED_MODEL.to_owned(),
                provider: provider.clone(),
            }),
        ),
        FileReadError::Timeout(_) | FileReadError::UnknownProvider { .. } => (
            Failure {
                class: "ValueError".to_owned(),
                status: 0,
                kind: None,
                param: None,
                code: None,
                message: error.to_string(),
            },
            None,
        ),
        FileReadError::Dispatch(wire) => (status_failure(wire)?, None),
    };
    Ok(StepOutcome {
        file: None,
        listing: None,
        failure: Some(failure),
        placeholder,
    })
}

fn status_failure(error: &ReadWireError) -> Result<Failure, SdkHttpError> {
    let status: &StatusError = match error {
        ReadWireError::OpenAi(FilesError::Status(status))
        | ReadWireError::Azure(AzureFilesError::Files {
            source: FilesError::Status(status),
        }) => status.as_ref(),
        ReadWireError::OpenAi(_)
        | ReadWireError::Azure(_)
        | ReadWireError::Anthropic(_)
        | ReadWireError::Unbuilt { .. }
        | ReadWireError::Reply
        | ReadWireError::Header { .. } => {
            return Err(SdkHttpError::Invalid(error.to_string()));
        }
    };
    Ok(Failure {
        class: sdk_class(status.status).to_owned(),
        status: status.status,
        kind: Some(status.error_type.clone()),
        param: Some(status.param.clone()),
        code: Some(status.code.clone()),
        message: status.message.clone(),
    })
}
