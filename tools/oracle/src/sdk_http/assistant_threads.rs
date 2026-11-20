use std::{collections::BTreeMap, sync::Arc};

use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::azure_endpoints;
use betterlite_core::api::assistant_threads::{
    AssistantThreads, CreateThreadCall, GetThreadCall, ThreadFacade as _,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_openai::threads::records::{ThreadMessageData, ThreadRecord};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::vector_query_embedding::outbound_client;

mod failure;

use failure::{async_failure, sync_failure};

// The facade pair create_thread/get_thread; the guard and flag rules live in betterlite-core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Create,
    Get,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Async,
}

// One facade call: the provider, the arguments the wrapper takes, and the kwargs it forwards.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub operation: Operation,
    pub mode: Mode,
    pub provider: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub messages: Option<Vec<ThreadMessageData>>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, Json>>,
    #[serde(default)]
    pub tool_resources: Option<Json>,
    #[serde(default)]
    pub kwargs: BTreeMap<String, Json>,
}

// A thread the provider returned, or null when the SDK returned nothing at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub thread: Option<ThreadRecord>,
}

enum Facade<'call> {
    Create(CreateThreadCall<'call, ProxiedClient>),
    Get(GetThreadCall<'call, ProxiedClient>),
}

impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        let resources = resources(env);
        let azure = azure_endpoints::Settings::default();
        let sources = NoSources;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let threads = AssistantThreads {
            transport: Arc::new(outbound_client(env)?),
            resources: &resources,
            azure: &azure,
            environment: env,
            sources: &sources,
            redactor: &redactor,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
        };
        // the harness owns the mock origin, so it takes the place of the configured api_base
        let mut kwargs = self.kwargs.clone();
        kwargs.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
        let facade = prepare(self, &mut kwargs)?;
        match (self.mode, facade) {
            (Mode::Sync, Facade::Create(call)) => match threads.create_thread(call).await {
                Ok(thread) => Ok(thread_outcome(thread)),
                Err(error) => Ok(sync_failure(&error)),
            },
            (Mode::Sync, Facade::Get(call)) => match threads.get_thread(call).await {
                Ok(thread) => Ok(thread_outcome(thread)),
                Err(error) => Ok(sync_failure(&error)),
            },
            (Mode::Async, Facade::Create(call)) => match threads.acreate_thread(call).await {
                Ok(thread) => Ok(thread_outcome(thread)),
                Err(failure) => async_failure(&failure),
            },
            (Mode::Async, Facade::Get(call)) => match threads.aget_thread(call).await {
                Ok(thread) => Ok(thread_outcome(thread)),
                Err(failure) => async_failure(&failure),
            },
        }
    }
}

fn prepare<'call>(
    call: &'call Call,
    kwargs: &'call mut BTreeMap<String, Json>,
) -> Result<Facade<'call>, SdkHttpError> {
    match call.operation {
        Operation::Create => Ok(Facade::Create(CreateThreadCall {
            custom_llm_provider: call.provider.clone(),
            messages: call.messages.clone(),
            metadata: call.metadata.clone(),
            tool_resources: call.tool_resources.clone(),
            client: None,
            kwargs,
        })),
        Operation::Get => {
            let thread_id = call.thread_id.clone().ok_or_else(|| {
                SdkHttpError::Invalid(String::from("a get call needs a thread_id"))
            })?;
            Ok(Facade::Get(GetThreadCall {
                custom_llm_provider: call.provider.clone(),
                thread_id,
                client: None,
                kwargs,
            }))
        }
    }
}

fn thread_outcome(thread: Option<ThreadRecord>) -> SdkResult {
    SdkResult::AssistantThreads {
        value: Box::new(Outcome { thread }),
    }
}

// The B2487 settings the server loads from config and environment for these two facades.
fn resources(env: &BTreeMap<String, String>) -> ResourceSdkSettings {
    ResourceSdkSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: env.get("AZURE_AD_TOKEN").cloned().map(SecretString::from),
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: env.get("OPENAI_BASE_URL").cloned(),
        openai_api_base: env.get("OPENAI_API_BASE").cloned(),
        openai_api_key: env.get("OPENAI_API_KEY").cloned().map(SecretString::from),
        openai_organization: env.get("OPENAI_ORGANIZATION").cloned(),
        azure_api_base: env.get("AZURE_API_BASE").cloned(),
        azure_api_version: env.get("AZURE_API_VERSION").cloned(),
        azure_openai_api_key: env
            .get("AZURE_OPENAI_API_KEY")
            .cloned()
            .map(SecretString::from),
        azure_api_key: env.get("AZURE_API_KEY").cloned().map(SecretString::from),
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}
