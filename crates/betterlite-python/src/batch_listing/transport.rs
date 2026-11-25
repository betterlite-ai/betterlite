use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::xai_identity;
use betterlite_core::api::batch_provider_cancel::{
    BatchCancelDispatch, CancelTarget, OpenAiCancelCall, XaiCancelCall,
};
use betterlite_core::api::batch_provider_list::BatchListProvider;
use betterlite_core::api::batch_provider_list::call::{
    BatchListProviderCall, BatchListWindow, BatchProviderListCall, OpenAiBatchListCall,
};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::batches::error::{BatchesError, SdkError};
use betterlite_openai::batches::http::HttpConstructor;
use betterlite_openai::batches::records::{BatchObject, BatchPage, BatchQuery};
use betterlite_openai::batches::sdk::{ClientSettings, Connection, Timeout, TimeoutPhases};
use betterlite_openai::batches::{BatchOperation, BatchesApi};
use betterlite_openai::xai_batches::error::XaiBatchesError;
use betterlite_openai::xai_batches::execution::{CallContext, XaiBatches};
use betterlite_openai::xai_batches::transformation::OpenAiBatchListResponse;
use serde::Serialize;

use super::error::{DispatchFailure, ErrorBody};

const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

// the BatchesApi both OpenAI entry points build over the shared pooled client
fn openai_batches(client: &ProxiedClient) -> BatchesApi<HttpConstructor> {
    BatchesApi {
        constructor: HttpConstructor {
            transport: client.clone(),
            environment_key: None,
            environment_organization: None,
            environment_project: None,
        },
    }
}

// the page a list call answered with, in the shape its own provider returns
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum ListAnswer {
    OpenAi(BatchPage),
    Xai(OpenAiBatchListResponse),
}

// The provider entry points list_batches dispatches to: the two executors this build has, and the rest named.
pub(crate) struct ListTransport {
    pub(crate) client: ProxiedClient,
    pub(crate) xai: xai_identity::Settings,
    pub(crate) limits: WireDecodeLimits,
}

impl ListTransport {
    pub(crate) async fn list(
        &self,
        call: &BatchProviderListCall,
        is_async: bool,
    ) -> Result<ListAnswer, DispatchFailure> {
        let window = &call.window;
        match &call.provider {
            BatchListProviderCall::OpenAiCompatible(inner) => {
                openai_list(self, inner, window, is_async)
                    .await
                    .map(ListAnswer::OpenAi)
            }
            // XAIBatchesHandler.list_batches
            BatchListProviderCall::Xai(inner) => {
                let timeout = duration_of(&window.timeout)?;
                let batches = XaiBatches {
                    transport: self.client.clone(),
                    settings: self.xai.clone(),
                    limits: self.limits,
                };
                let context = CallContext {
                    api_base: inner.api_base.as_deref(),
                    api_key: inner.api_key.as_ref(),
                    timeout,
                };
                let page = batches
                    .list_batches(context, window.after.as_deref(), window.limit)
                    .await
                    .map_err(|error| dispatch_failure(ProviderFailure::Xai(&error)))?;
                Ok(ListAnswer::Xai(page))
            }
            BatchListProviderCall::Azure(_) => Err(DispatchFailure::Unsupported {
                provider: BatchListProvider::Azure.name().to_owned(),
            }),
            BatchListProviderCall::VertexAi(_) => Err(DispatchFailure::Unsupported {
                provider: BatchListProvider::VertexAi.name().to_owned(),
            }),
        }
    }
}

// OpenAIBatchesAPI.list_batches
async fn openai_list(
    transport: &ListTransport,
    call: &OpenAiBatchListCall,
    window: &BatchListWindow,
    is_async: bool,
) -> Result<BatchPage, DispatchFailure> {
    let connection = Connection {
        is_async,
        settings: ClientSettings {
            api_key: call.api_key.clone(),
            api_base: Some(call.api_base.clone()),
            timeout: Some(window.timeout.clone()),
            max_retries: call.max_retries,
            organization: call.organization.clone(),
        },
        client: None,
    };
    openai_batches(&transport.client)
        .list_batches(connection, window.after.as_deref(), window.limit)
        .await
        .map_err(|error| dispatch_failure(ProviderFailure::OpenAi(&error)))
}

// The five provider entry points cancel_batch dispatches to: the two executors this build has, and the rest named.
pub(crate) struct CancelDispatch {
    pub(crate) client: ProxiedClient,
    pub(crate) xai: xai_identity::Settings,
    pub(crate) limits: WireDecodeLimits,
}

impl BatchCancelDispatch for CancelDispatch {
    type Error = DispatchFailure;

    fn cancel(
        &self,
        target: CancelTarget<'_>,
    ) -> impl Future<Output = Result<BatchObject, DispatchFailure>> + Send {
        let owned = target.clone();
        let client = self.client.clone();
        let xai = self.xai.clone();
        let limits = self.limits;
        async move {
            match owned {
                CancelTarget::OpenAi(call) => openai_cancel(&client, &call).await,
                CancelTarget::Xai(call) => xai_cancel(&client, &xai, limits, &call).await,
                CancelTarget::Azure(_) => Err(DispatchFailure::Unsupported {
                    provider: BatchListProvider::Azure.name().to_owned(),
                }),
                CancelTarget::Vertex(_) => Err(DispatchFailure::Unsupported {
                    provider: BatchListProvider::VertexAi.name().to_owned(),
                }),
                CancelTarget::Bedrock(_) => Err(DispatchFailure::Unsupported {
                    provider: "bedrock".to_owned(),
                }),
            }
        }
    }
}

// OpenAIBatchesAPI.cancel_batch, whose request is the batch id the provider branch read
async fn openai_cancel(
    client: &ProxiedClient,
    call: &OpenAiCancelCall<'_>,
) -> Result<BatchObject, DispatchFailure> {
    let values = call.values;
    let query = BatchQuery {
        batch_id: values.batch_id.to_owned(),
        extra_headers: values.request.extra_headers.clone(),
        extra_body: values.request.extra_body.clone(),
        timeout: None,
    };
    let connection = Connection {
        is_async: values.is_async,
        settings: ClientSettings {
            api_key: call.api_key.cloned(),
            api_base: Some(call.api_base.clone()),
            timeout: Some(sdk_timeout(values.timeout)),
            max_retries: values.max_retries,
            organization: call.organization.map(str::to_owned),
        },
        client: None,
    };
    openai_batches(client)
        .batch(connection, BatchOperation::Cancel(&query))
        .await
        .map_err(|error| dispatch_failure(ProviderFailure::OpenAi(&error)))
}

// XAIBatchesHandler.cancel_batch
async fn xai_cancel(
    client: &ProxiedClient,
    settings: &xai_identity::Settings,
    limits: WireDecodeLimits,
    call: &XaiCancelCall<'_>,
) -> Result<BatchObject, DispatchFailure> {
    let timeout = duration_of(&sdk_timeout(call.values.timeout))?;
    let batches = XaiBatches {
        transport: client.clone(),
        settings: settings.clone(),
        limits,
    };
    let context = CallContext {
        api_base: call.api_base,
        api_key: call.api_key,
        timeout,
    };
    batches
        .cancel_batch(call.values.batch_id, context)
        .await
        .map_err(|error| dispatch_failure(ProviderFailure::Xai(&error)))
}

// the SDK error a provider call answered with, in its executor's own type
#[derive(Clone, Copy)]
pub(crate) enum ProviderFailure<'error> {
    OpenAi(&'error BatchesError),
    Xai(&'error XaiBatchesError),
}

// one shape for both executors: a provider status keeps its own message, anything else is a connection failure
const fn failure(status: Option<u16>, message: String) -> DispatchFailure {
    match status {
        Some(code) => DispatchFailure::Status {
            status: code,
            message,
            body: None,
        },
        None => DispatchFailure::Connection(message),
    }
}

// an SDK status keeps the provider's message and error object; every other SDK failure never reached it
pub(crate) fn dispatch_failure(error: ProviderFailure<'_>) -> DispatchFailure {
    match error {
        ProviderFailure::OpenAi(BatchesError::Sdk {
            source:
                SdkError::Status {
                    status,
                    error_type,
                    param,
                    code,
                    message,
                },
        }) => DispatchFailure::Status {
            status: *status,
            message: message.clone(),
            body: ErrorBody::from_fields(error_type.clone(), param.clone(), code.clone()),
        },
        ProviderFailure::OpenAi(sdk) => failure(None, sdk.to_string()),
        ProviderFailure::Xai(sdk) => match sdk {
            XaiBatchesError::Status {
                status, message, ..
            } => failure(Some(*status), message.clone()),
            XaiBatchesError::MissingApiKey => failure(Some(401), sdk.to_string()),
            XaiBatchesError::InputFileRequired => failure(Some(400), sdk.to_string()),
            XaiBatchesError::Limit(_)
            | XaiBatchesError::Decode { .. }
            | XaiBatchesError::Transport { .. }
            | XaiBatchesError::Path { .. }
            | XaiBatchesError::Authorization { .. }
            | XaiBatchesError::BaseUrl { .. }
            | XaiBatchesError::Encode { .. } => failure(None, sdk.to_string()),
        },
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub(crate) enum DurationError {
    #[error("the timeout must be a finite, non-negative number of seconds, not {seconds}")]
    Invalid { seconds: f64 },
}

// a timeout as the executor's deadline: a phase object contributes its read phase, or the 10 minute default
pub(crate) fn duration_of(timeout: &Timeout) -> Result<Duration, DurationError> {
    let seconds = match timeout {
        Timeout::Seconds(seconds) => *seconds,
        Timeout::Phases(phases) => phases.read.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
    };
    Duration::try_from_secs_f64(seconds).map_err(|_source| DurationError::Invalid { seconds })
}

// the timeout an OpenAI SDK client takes: the seconds, or the phases it would apply per request
pub(crate) const fn sdk_timeout(timeout: &FineTuningTimeout) -> Timeout {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => Timeout::Seconds(*seconds),
        FineTuningTimeout::Structured(phases) => Timeout::Phases(TimeoutPhases {
            connect: phases.connect,
            read: phases.read,
            write: phases.write,
            pool: phases.pool,
        }),
    }
}
