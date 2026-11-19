pub mod error;
mod params;
mod runtime;
mod target;
mod tests;

use std::collections::BTreeMap;
use std::fmt;
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

use self::error::{ThreadError, ThreadFailure, failure};
use self::runtime::{SdkThreadRuntime, ThreadRuntime};
use self::target::{Operation, PrepareRequest, create_body};

// the two facades differ only in their name inside the unsupported-provider message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadOperation {
    Create,
    Get,
}

impl fmt::Display for ThreadOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create => formatter.write_str("create_thread"),
            Self::Get => formatter.write_str("get_thread"),
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

impl<'deps, T> AssistantThreads<'deps, T>
where
    T: AssistantsTransport,
{
    pub async fn create_thread<'call>(
        &self,
        call: CreateThreadCall<'call, T>,
    ) -> Result<Option<ThreadRecord>, ThreadError>
    where
        'deps: 'call,
    {
        self.create_thread_with(call, &SdkThreadRuntime).await
    }

    pub async fn acreate_thread<'call>(
        &self,
        mut call: CreateThreadCall<'call, T>,
    ) -> Result<Option<ThreadRecord>, ThreadFailure>
    where
        'deps: 'call,
    {
        call.kwargs
            .insert("acreate_thread".to_owned(), Json::Bool(true));
        let provider = call.custom_llm_provider.clone();
        match self.create_thread_with(call, &SdkThreadRuntime).await {
            Ok(thread) => Ok(thread),
            Err(error) => Err(failure(error, &provider, self.redactor)),
        }
    }

    pub async fn get_thread<'call>(
        &self,
        call: GetThreadCall<'call, T>,
    ) -> Result<Option<ThreadRecord>, ThreadError>
    where
        'deps: 'call,
    {
        self.get_thread_with(call, &SdkThreadRuntime).await
    }

    pub async fn aget_thread<'call>(
        &self,
        mut call: GetThreadCall<'call, T>,
    ) -> Result<Option<ThreadRecord>, ThreadFailure>
    where
        'deps: 'call,
    {
        call.kwargs
            .insert("aget_thread".to_owned(), Json::Bool(true));
        let provider = call.custom_llm_provider.clone();
        match self.get_thread_with(call, &SdkThreadRuntime).await {
            Ok(thread) => Ok(thread),
            Err(error) => Err(failure(error, &provider, self.redactor)),
        }
    }

    async fn create_thread_with<'call, Runtime>(
        &self,
        call: CreateThreadCall<'call, T>,
        runtime: &Runtime,
    ) -> Result<Option<ThreadRecord>, ThreadError>
    where
        'deps: 'call,
        Runtime: ThreadRuntime<T>,
    {
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
        let prepared = self.prepare(PrepareRequest {
            provider: custom_llm_provider,
            operation: Operation::Create(create_body(messages, metadata)?),
            client,
            kwargs: &*kwargs,
            facade: ThreadOperation::Create,
            flag,
        })?;
        runtime.run(prepared).await
    }

    async fn get_thread_with<'call, Runtime>(
        &self,
        call: GetThreadCall<'call, T>,
        runtime: &Runtime,
    ) -> Result<Option<ThreadRecord>, ThreadError>
    where
        'deps: 'call,
        Runtime: ThreadRuntime<T>,
    {
        let GetThreadCall {
            custom_llm_provider,
            thread_id,
            client,
            kwargs,
        } = call;
        let flag = kwargs
            .remove("aget_thread")
            .map(|value| matches!(value, Json::Bool(true)));
        let prepared = self.prepare(PrepareRequest {
            provider: custom_llm_provider,
            operation: Operation::Retrieve(thread_id),
            client,
            kwargs: &*kwargs,
            facade: ThreadOperation::Get,
            flag,
        })?;
        runtime.run(prepared).await
    }
}
