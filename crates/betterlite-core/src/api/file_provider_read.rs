use std::collections::BTreeMap;

use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::call_params::TrustedCredentials;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use uuid::Uuid;

use crate::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};
use crate::api::vector_store_http_write::service::SuppliedClient;

use self::error::{FileReadError, ReadFunction};
use self::plan::{Plan, known_provider, read_timeout};
use self::target::{
    FileReadDispatch, GenericRead, ReadAnswer, ReadLogger, ReadMode, ReadOperation, ReadTarget,
};

pub mod error;
mod plan;
pub mod target;
mod tests;

// OPENAI_COMPATIBLE_BATCH_AND_FILES_PROVIDERS: these names share the OpenAI files client
const COMPATIBLE_PROVIDERS: [&str; 3] = ["openai", "hosted_vllm", "litellm_proxy"];

// A kwarg LiteLLM reads with kwargs.get, where a None the caller passed is not a missing key
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Kwarg {
    #[default]
    Absent,
    Null,
    Text(String),
}

// The provider literal both functions declare; any other text reaches the branch that reads it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadProvider(pub String);

impl Default for ReadProvider {
    fn default() -> Self {
        Self("openai".to_owned())
    }
}

// The kwargs file_retrieve and file_list read, beside the declared arguments
#[derive(Debug, Clone, Default)]
pub struct ReadKwargs {
    pub custom_llm_provider: ReadProvider,
    pub extra_headers: Option<BTreeMap<String, String>>,
    // declared by both functions and forwarded by neither
    pub extra_body: Option<BTreeMap<String, String>>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<ReadTimeout>,
    pub request_timeout: Option<ReadTimeoutValue>,
    pub max_retries: Option<i64>,
    pub is_async: bool,
    pub litellm_call_id: Kwarg,
    pub id: Kwarg,
    // the cursor a caller may pass to list; no branch of the dispatcher forwards it
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub order: Option<String>,
}

pub enum CallerClient<'call, Client> {
    // HTTPHandler or AsyncHTTPHandler, the only classes the generic handler keeps
    Http(SuppliedClient<'call, Client>),
    // an OpenAI or Azure SDK client, which none of the four functions forwards
    Sdk,
}

// What only server code hands over: the trusted credential snapshot, the caller's client and its logging object
pub struct ReadContext<'call, Client> {
    pub trusted: Option<&'call TrustedCredentials>,
    pub client: Option<CallerClient<'call, Client>>,
    pub logger: Option<&'call mut LoggingState>,
}

// file_retrieve or afile_retrieve, file_list or afile_list: the async wrapper sets is_async before the sync body runs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadEntry {
    Sync,
    Async,
}

pub struct FileProviderRead<Dispatch> {
    pub dispatch: Dispatch,
    pub settings: Settings,
    pub clock: fn() -> DateTime<FixedOffset>,
    pub call_ids: fn() -> Uuid,
}

type Reply<Value, Dispatch> = Result<Value, FileReadError<<Dispatch as FileReadDispatch>::Error>>;

impl<Dispatch> FileProviderRead<Dispatch>
where
    Dispatch: FileReadDispatch,
{
    // LiteLLM's two orders: retrieve answers the OpenAI-compatible and azure names before it reads a provider config, list asks for one first
    pub async fn run(
        &self,
        entry: ReadEntry,
        operation: ReadOperation<'_>,
        kwargs: &ReadKwargs,
        context: ReadContext<'_, Dispatch::Client>,
    ) -> Reply<ReadAnswer, Dispatch> {
        let plan = self.plan(operation.function(), entry, kwargs)?;
        let provider = kwargs.custom_llm_provider.0.as_str();
        let target = match operation {
            ReadOperation::Retrieve { .. } => match named_target(&plan, provider) {
                Some(named) => named,
                None => ReadTarget::from(self.generic(
                    &plan,
                    self.config(provider, plan.function)?,
                    context,
                )),
            },
            ReadOperation::List { .. } => {
                match self.dispatch.files_config(known_provider(provider)?) {
                    Some(config) => ReadTarget::from(self.generic(&plan, config, context)),
                    None => {
                        named_target(&plan, provider).ok_or_else(|| FileReadError::Unsupported {
                            provider: provider.to_owned(),
                            function: plan.function,
                        })?
                    }
                }
            }
        };
        self.dispatch
            .read(target, operation)
            .await
            .map_err(FileReadError::Dispatch)
    }

    // The timeout and the mode are settled before the provider is looked at, so a bad timeout fails first
    fn plan<'call>(
        &self,
        function: ReadFunction,
        entry: ReadEntry,
        kwargs: &'call ReadKwargs,
    ) -> Reply<Plan<'call>, Dispatch> {
        let timeout = read_timeout(kwargs, &kwargs.custom_llm_provider.0)?;
        Ok(Plan {
            function,
            kwargs,
            mode: ReadMode {
                is_async: entry == ReadEntry::Async || kwargs.is_async,
                timeout,
            },
            settings: ResourceSdkSettings::from(&self.settings),
            options: ResourceSdkRequestOptions {
                api_base: kwargs.api_base.clone(),
                api_version: kwargs.api_version.clone(),
                api_key: kwargs.api_key.clone(),
                organization: kwargs.organization.clone(),
                vertex_project: None,
                vertex_location: None,
                vertex_credentials: None,
            },
        })
    }

    // the provider's files config, or the 400 LiteLLM raises when the registry has none for it
    fn config(&self, provider: &str, function: ReadFunction) -> Reply<Dispatch::Config, Dispatch> {
        self.dispatch
            .files_config(known_provider(provider)?)
            .ok_or_else(|| FileReadError::Unsupported {
                provider: provider.to_owned(),
                function,
            })
    }

    // get_litellm_params' api_key and api_base are written over with the call's own values, even when those are None
    fn generic<'call>(
        &self,
        plan: &Plan<'_>,
        config: Dispatch::Config,
        context: ReadContext<'call, Dispatch::Client>,
    ) -> GenericRead<'call, Dispatch::Config, Dispatch::Client> {
        let ReadContext {
            trusted,
            client,
            logger,
        } = context;
        GenericRead {
            config,
            api_key: plan.kwargs.api_key.clone(),
            api_base: plan.kwargs.api_base.clone(),
            trusted,
            headers: plan.kwargs.extra_headers.clone().unwrap_or_default(),
            logger: logger.map_or_else(
                || {
                    let call_id = (self.call_ids)().to_string();
                    ReadLogger::Created(Box::new(plan.new_logger((self.clock)(), call_id)))
                },
                ReadLogger::Supplied,
            ),
            client: match client {
                Some(CallerClient::Http(supplied)) => Some(supplied),
                Some(CallerClient::Sdk) | None => None,
            },
            mode: plan.mode,
        }
    }
}

// The two names retrieve answers before it asks for a provider config, and that list falls back to after one
fn named_target<'call, Dispatch>(
    plan: &Plan<'_>,
    provider: &str,
) -> Option<ReadTarget<'call, Dispatch::Config, Dispatch::Client>>
where
    Dispatch: FileReadDispatch,
{
    if COMPATIBLE_PROVIDERS.contains(&provider) {
        Some(ReadTarget::Compatible(plan.compatible()))
    } else if provider == "azure" {
        Some(ReadTarget::Azure(plan.azure()))
    } else {
        None
    }
}
