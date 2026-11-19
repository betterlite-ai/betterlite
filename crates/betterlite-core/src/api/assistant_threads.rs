pub mod error;
mod params;
mod runtime;
mod target;
mod tests;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::credentials::source::CredentialSources;
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints;
use betterlite_models::decisions::Json;
use betterlite_openai::assistants::client::SdkClient;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{ThreadMessageData, ThreadRecord};
use betterlite_redact::secret::Redactor;

use crate::api::resource_sdk_settings::ResourceSdkSettings;

use self::error::{ThreadError, ThreadFailure};
use self::runtime::{SdkThreadRuntime, ThreadRuntime};
use self::target::{Operation, PrepareRequest, create_body};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadOperation {
    Create,
    Get,
}

impl ThreadOperation {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Create => "create_thread",
            Self::Get => "get_thread",
        }
    }
}

// A supplied client's kind is its SDK class: sync is OpenAI or AzureOpenAI, async is AsyncOpenAI or AsyncAzureOpenAI.
pub enum SuppliedClient<'call, T> {
    Openai(SdkClient<T>),
    Azure(AzureAssistantsClient<'call, T>),
}

pub struct CreateThreadCall<'call, T> {
    pub custom_llm_provider: String,
    pub messages: Option<Vec<ThreadMessageData>>,
    pub metadata: Option<BTreeMap<String, Json>>,
    // accepted for signature parity and never forwarded, on either provider
    pub tool_resources: Option<Json>,
    pub client: Option<SuppliedClient<'call, T>>,
    // the caller's kwargs: the flag is read in place, and acreate_thread writes it before delegating
    pub kwargs: &'call mut BTreeMap<String, Json>,
}

pub struct GetThreadCall<'call, T> {
    pub custom_llm_provider: String,
    pub thread_id: String,
    pub client: Option<SuppliedClient<'call, T>>,
    // get_thread pops aget_thread from these kwargs before anything else reads them
    pub kwargs: &'call mut BTreeMap<String, Json>,
}

pub struct AssistantThreads<'deps, T> {
    pub transport: Arc<T>,
    pub resources: &'deps ResourceSdkSettings,
    pub azure: &'deps azure_endpoints::Settings,
    pub environment: &'deps (dyn EnvironmentLookup + Sync),
    pub sources: &'deps (dyn CredentialSources + Sync),
    pub redactor: &'deps Redactor,
    pub refresh_enabled: bool,
    pub session: SessionKind,
}

type ThreadResult = Result<Option<ThreadRecord>, ThreadError>;
type FailureResult = Result<Option<ThreadRecord>, ThreadFailure>;

enum FacadeCall<'call, T> {
    Create(CreateThreadCall<'call, T>),
    Get(GetThreadCall<'call, T>),
}

pub trait ThreadFacade<'deps, T> {
    fn create_thread<'call>(
        &self,
        call: CreateThreadCall<'call, T>,
    ) -> impl Future<Output = ThreadResult>
    where
        'deps: 'call;

    fn acreate_thread<'call>(
        &self,
        call: CreateThreadCall<'call, T>,
    ) -> impl Future<Output = FailureResult>
    where
        'deps: 'call;

    fn get_thread<'call>(
        &self,
        call: GetThreadCall<'call, T>,
    ) -> impl Future<Output = ThreadResult>
    where
        'deps: 'call;

    fn aget_thread<'call>(
        &self,
        call: GetThreadCall<'call, T>,
    ) -> impl Future<Output = FailureResult>
    where
        'deps: 'call;
}

impl<'deps, T> ThreadFacade<'deps, T> for AssistantThreads<'deps, T>
where
    T: AssistantsTransport,
{
    async fn create_thread<'call>(&self, call: CreateThreadCall<'call, T>) -> ThreadResult
    where
        'deps: 'call,
    {
        execute(self, FacadeCall::Create(call), &SdkThreadRuntime).await
    }

    async fn acreate_thread<'call>(&self, call: CreateThreadCall<'call, T>) -> FailureResult
    where
        'deps: 'call,
    {
        map_failure(self, FacadeCall::Create(call), "acreate_thread").await
    }

    async fn get_thread<'call>(&self, call: GetThreadCall<'call, T>) -> ThreadResult
    where
        'deps: 'call,
    {
        execute(self, FacadeCall::Get(call), &SdkThreadRuntime).await
    }

    async fn aget_thread<'call>(&self, call: GetThreadCall<'call, T>) -> FailureResult
    where
        'deps: 'call,
    {
        map_failure(self, FacadeCall::Get(call), "aget_thread").await
    }
}

fn create_request<'call, T>(
    call: CreateThreadCall<'call, T>,
) -> Result<PrepareRequest<'call, T>, ThreadError> {
    let CreateThreadCall {
        custom_llm_provider,
        messages,
        metadata,
        client,
        kwargs,
        ..
    } = call;
    let flag = kwargs
        .get("acreate_thread")
        .map(|value| matches!(value, Json::Bool(true)));
    Ok(PrepareRequest {
        provider: custom_llm_provider,
        operation: Operation::Create(create_body(messages, metadata)?),
        client,
        kwargs: &*kwargs,
        facade: ThreadOperation::Create,
        flag,
    })
}

// one implementation for both operations; the runtime stays a parameter so tests capture the prepared call
async fn execute<'deps, 'call, T, Runtime>(
    threads: &AssistantThreads<'deps, T>,
    call: FacadeCall<'call, T>,
    runtime: &Runtime,
) -> ThreadResult
where
    T: AssistantsTransport,
    Runtime: ThreadRuntime<T> + Sync,
    'deps: 'call,
{
    let prepared = match call {
        FacadeCall::Create(create) => threads.prepare(create_request(create)?)?,
        FacadeCall::Get(get) => {
            let GetThreadCall {
                custom_llm_provider,
                thread_id,
                client,
                kwargs,
            } = get;
            let flag = kwargs
                .remove("aget_thread")
                .map(|value| matches!(value, Json::Bool(true)));
            threads.prepare(PrepareRequest {
                provider: custom_llm_provider,
                operation: Operation::Retrieve(thread_id),
                client,
                kwargs: &*kwargs,
                facade: ThreadOperation::Get,
                flag,
            })?
        }
    };
    runtime.run(prepared).await
}

async fn map_failure<'deps, 'call, T>(
    threads: &AssistantThreads<'deps, T>,
    mut call: FacadeCall<'call, T>,
    flag: &str,
) -> FailureResult
where
    T: AssistantsTransport,
    'deps: 'call,
{
    let provider = match &call {
        FacadeCall::Create(create) => create.custom_llm_provider.clone(),
        FacadeCall::Get(get) => get.custom_llm_provider.clone(),
    };
    let kwargs = match &mut call {
        FacadeCall::Create(create) => &mut *create.kwargs,
        FacadeCall::Get(get) => &mut *get.kwargs,
    };
    kwargs.insert(flag.to_owned(), Json::Bool(true));
    execute(threads, call, &SdkThreadRuntime)
        .await
        .map_err(|error| ThreadFailure {
            class: error.error_class(&provider, threads.redactor),
            error,
        })
}
