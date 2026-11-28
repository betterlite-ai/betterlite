use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::azure_assistant_definitions::operations::ClientRequest;
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::credentials::source::CredentialSources;
use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::thread_runs::records::{RunReply, RunRequest};
use betterlite_openai::thread_runs::stream::RunEventHandler;
use betterlite_redact::secret::Redactor;

use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::call::RunClientClass::{AsyncAzure, Azure};
use super::call::{RunClient, RunFlag, RunThreadCall};
use super::convert::{
    azure_timeout, client_kind, facade_timeout, facade_token, openai_options, run_request,
    supplied_timeout,
};
use super::error::{AssistantRunError, failed, map_failure};

pub trait RunOperations: Send + Sync {
    type Stream;

    fn poll(
        &self,
        target: RunTarget<'_>,
    ) -> impl Future<Output = Result<Option<RunReply>, AssistantsRunError>> + Send;

    fn stream(&self, target: RunTarget<'_>) -> Result<Self::Stream, AssistantsRunError>;
}

pub struct OpenAiRun {
    pub request: RunRequest,
    pub kind: ClientKind,
    pub handler: Option<Arc<dyn RunEventHandler>>,
    pub options: ClientOptions,
    pub supplied: Option<RunClient>,
    pub supplied_timeout: Option<Duration>,
}

pub struct AzureRun<'data> {
    pub request: RunRequest,
    pub kind: ClientKind,
    pub handler: Option<Arc<dyn RunEventHandler>>,
    pub client: ClientRequest<'data>,
    pub supplied: Option<RunClient>,
    pub supplied_timeout: Option<Duration>,
}

pub enum RunTarget<'data> {
    OpenAi(Box<OpenAiRun>),
    Azure(Box<AzureRun<'data>>),
}

pub enum RunOutcome<S> {
    Polled(Box<Option<RunReply>>),
    Streaming(S),
}

pub struct AssistantRunSdk<'data, O> {
    pub operations: O,
    pub settings: &'data ResourceSdkSettings,
    pub azure: &'data AzureSettings,
    pub environment: &'data (dyn EnvironmentLookup + Sync),
    pub sources: &'data (dyn CredentialSources + Sync),
    pub redactor: &'data Redactor,
}

impl<'data, O> AssistantRunSdk<'data, O>
where
    O: RunOperations,
{
    pub async fn run_thread(
        &self,
        call: RunThreadCall,
    ) -> Result<RunOutcome<O::Stream>, AssistantRunError> {
        let target = self.prepare(&call)?;
        if matches!(call.stream, Some(Some(RunFlag::Bool(true)))) {
            return self
                .operations
                .stream(target)
                .map(RunOutcome::Streaming)
                .map_err(AssistantRunError::Dispatch);
        }
        self.operations
            .poll(target)
            .await
            .map(|reply| RunOutcome::Polled(Box::new(reply)))
            .map_err(AssistantRunError::Dispatch)
    }

    // run_thread_stream(*, event_handler=None, **kwargs) -> run_thread(stream=True, ...)
    pub fn run_thread_stream(
        &self,
        mut call: RunThreadCall,
    ) -> Result<O::Stream, AssistantRunError> {
        if call.stream.is_some() {
            return Err(AssistantRunError::DuplicateStream);
        }
        call.stream = Some(Some(RunFlag::Bool(true)));
        let target = self.prepare(&call)?;
        self.operations
            .stream(target)
            .map_err(AssistantRunError::Dispatch)
    }

    // arun_thread_stream is itself synchronous: it forces the switch and returns the async manager
    pub fn arun_thread_stream(
        &self,
        mut call: RunThreadCall,
    ) -> Result<O::Stream, AssistantRunError> {
        call.arun_thread = Some(RunFlag::Bool(true));
        self.run_thread_stream(call)
    }

    pub async fn arun_thread(
        &self,
        mut call: RunThreadCall,
    ) -> Result<RunOutcome<O::Stream>, AssistantRunError> {
        call.arun_thread = Some(RunFlag::Bool(true));
        self.run_arun(call).await
    }

    async fn run_arun(
        &self,
        call: RunThreadCall,
    ) -> Result<RunOutcome<O::Stream>, AssistantRunError> {
        let provider = call.provider.clone();
        self.run_thread(call)
            .await
            .map_err(|error| self.map_arun_error(&provider, error))
    }

    // arun_thread's except block: provider and wrapper failures are mapped, a litellm error passes through
    fn map_arun_error(&self, provider: &str, error: AssistantRunError) -> AssistantRunError {
        // LiteLLM's client raises APITimeoutError at once for the negative deadline it was handed
        let negative = matches!(error, AssistantRunError::InvalidTimeout { .. });
        let (text, status) = if negative {
            (String::from("Request timed out."), Some(408))
        } else {
            (error.to_string(), None)
        };
        match error {
            AssistantRunError::Dispatch(dispatch) => {
                AssistantRunError::Mapped(map_failure(provider, &dispatch, self.redactor))
            }
            AssistantRunError::Timeout(_)
            | AssistantRunError::InvalidTimeout { .. }
            | AssistantRunError::InvalidTimeoutText { .. }
            | AssistantRunError::InvalidExtraBody => {
                AssistantRunError::Mapped(failed(provider, text, status, None, self.redactor))
            }
            mapped @ (AssistantRunError::Unsupported { .. }
            | AssistantRunError::Mapped(_)
            | AssistantRunError::DuplicateStream) => mapped,
        }
    }

    // a supplied client stays the class it was built as, whatever provider the caller named
    fn prepare(&self, call: &RunThreadCall) -> Result<RunTarget<'data>, AssistantRunError> {
        let timeout = facade_timeout(call, &call.provider)?;
        let client_timeout = supplied_timeout(call)?;
        let arun = matches!(call.arun_thread, Some(RunFlag::Bool(true)));
        let kind = client_kind(call, arun);
        // main.py passes event_handler to the openai wrapper only
        let handler = match call.provider.as_str() {
            "openai" => call.event_handler.clone(),
            "azure" => None,
            other => {
                return Err(AssistantRunError::Unsupported {
                    provider: other.to_owned(),
                });
            }
        };
        let azure = call.client.as_ref().map_or_else(
            || call.provider.as_str() == "azure",
            |client| matches!(client.class, AsyncAzure | Azure),
        );
        if azure {
            let run = self.azure_run(call, kind, handler, timeout)?;
            return Ok(RunTarget::Azure(Box::new(run)));
        }
        Ok(RunTarget::OpenAi(Box::new(OpenAiRun {
            request: run_request(call),
            kind,
            handler,
            options: openai_options(call, self.settings, timeout),
            supplied: call.client.clone(),
            supplied_timeout: client_timeout,
        })))
    }

    // the azure provider branch is the only one that reads extra_body, stringifies and forwards litellm_params
    fn azure_run(
        &self,
        call: &RunThreadCall,
        kind: ClientKind,
        handler: Option<Arc<dyn RunEventHandler>>,
        timeout: Duration,
    ) -> Result<AzureRun<'data>, AssistantRunError> {
        let client_timeout = supplied_timeout(call)?;
        let options = ResourceSdkRequestOptions::from(&call.params);
        Ok(AzureRun {
            request: run_request(call),
            kind,
            handler,
            client: ClientRequest {
                params: LiteLlmParams {
                    azure_ad_token: call.params.azure_ad_token.clone(),
                    auth: AuthParams {
                        tenant_id: call.params.tenant_id.clone(),
                        client_id: call.params.client_id.clone(),
                        client_secret: call.params.client_secret.clone(),
                        azure_username: call.params.azure_username.clone(),
                        azure_password: call.params.azure_password.clone(),
                        azure_scope: call.params.azure_scope.clone(),
                    },
                    max_retries: call.params.max_retries,
                    timeout: azure_timeout(&call.params)?,
                },
                provider: None,
                api_key: self.settings.azure_api_key(&options).cloned(),
                api_base: self.settings.azure_base(&options).map(str::to_owned),
                api_version: self.settings.azure_version(&options).map(str::to_owned),
                azure_ad_token: facade_token(call, self.settings)?,
                timeout: timeout.as_secs_f64(),
                max_retries: call.params.max_retries,
                refresh_enabled: false,
                session: SessionKind::Dedicated,
                settings: self.azure,
                environment: self.environment,
                sources: self.sources,
            },
            supplied: call.client.clone(),
            supplied_timeout: client_timeout,
        })
    }
}
