use std::future::Future;

use betterlite_config::sections::xai_identity;
use betterlite_core::api::batch_provider_create::error::BatchProviderCreateError;
use betterlite_core::api::batch_provider_create::transports::BatchCreateTransport;
use betterlite_core::api::batch_provider_create::{
    BatchProvider, BatchProviderCreateCall, ProviderBatchCall,
};
use betterlite_core::api::batch_provider_read::call::{
    BatchProviderReadCall, OpenAiBatchReadCall, XaiBatchReadCall,
};
use betterlite_core::api::batch_provider_read::{BatchReadDispatch, BatchReadProvider};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::batches::error::{BatchesError, SdkError};
use betterlite_openai::batches::http::HttpConstructor;
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::batches::sdk::{ClientSettings, Connection, Timeout};
use betterlite_openai::batches::{BatchOperation, BatchesApi};
use betterlite_openai::xai_batches::error::XaiBatchesError;
use betterlite_openai::xai_batches::execution::{CallContext, XaiBatches};
use pyo3_async_runtimes::tokio::get_runtime;

mod errors;

use super::convert::request::{batch_create, batch_query, duration_of};
use super::error::DispatchFailure;

// the BatchesApi both OpenAI entry points build over the shared pooled client
fn openai_batches(client: &ProxiedClient) -> BatchesApi {
    BatchesApi {
        constructor: HttpConstructor {
            transport: client.clone(),
            environment_key: None,
            environment_organization: None,
            environment_project: None,
        },
    }
}

// OpenAIBatchesAPI's constructor settings for a create call
fn openai_connection(call: &ProviderBatchCall) -> Connection {
    Connection {
        is_async: call.is_async,
        settings: ClientSettings {
            api_key: call.api_key.clone(),
            api_base: call.api_base.clone(),
            timeout: Some(Timeout::Seconds(call.timeout)),
            max_retries: call.max_retries.map(i64::from),
            organization: call.organization.clone(),
        },
        client: None,
    }
}

// The four provider entry points create_batch calls, over the executors this build has.
pub(crate) struct BatchTransport {
    pub(crate) client: ProxiedClient,
    pub(crate) xai: xai_identity::Settings,
    pub(crate) limits: WireDecodeLimits,
}

impl BatchCreateTransport for BatchTransport {
    fn create(
        &self,
        call: &BatchProviderCreateCall,
    ) -> Result<BatchObject, BatchProviderCreateError> {
        match call {
            // OpenAIBatchesAPI.create_batch over the shared pooled client
            BatchProviderCreateCall::OpenAiCompatible(inner) => {
                let request = batch_create(&inner.body).map_err(|_| INVALID_CALL)?;
                let operation = BatchOperation::Create(&request);
                blocking_create(
                    openai_batches(&self.client).batch(openai_connection(inner), operation),
                    create_status,
                )
            }
            // XAIBatchesHandler.create_batch over the shared pooled client
            BatchProviderCreateCall::Xai(inner) => {
                let timeout =
                    duration_of(&Timeout::Seconds(inner.timeout)).map_err(|_| INVALID_CALL)?;
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
                blocking_create(batches.create_batch(&inner.body, context), xai_status)
            }
            BatchProviderCreateCall::ProviderConfig(inner) => {
                Err(BatchProviderCreateError::UnsupportedProvider {
                    provider: inner.provider,
                })
            }
            BatchProviderCreateCall::Azure(_) => {
                Err(BatchProviderCreateError::UnsupportedProvider {
                    provider: BatchProvider::Azure,
                })
            }
            BatchProviderCreateCall::VertexAi(_) => {
                Err(BatchProviderCreateError::UnsupportedProvider {
                    provider: BatchProvider::VertexAi,
                })
            }
        }
    }
}

// the request itself is what the provider would refuse, before any call goes out
const INVALID_CALL: BatchProviderCreateError = BatchProviderCreateError::Provider { status: 400 };

// both create executors wait on their own runtime and turn a failure into the status its class carries
fn blocking_create<Error, Fut>(
    future: Fut,
    status_of: fn(&Error) -> u16,
) -> Result<BatchObject, BatchProviderCreateError>
where
    Fut: Future<Output = Result<BatchObject, Error>>,
{
    get_runtime()
        .block_on(future)
        .map_err(|error| BatchProviderCreateError::Provider {
            status: status_of(&error),
        })
}

pub(crate) fn create_status(error: &BatchesError) -> u16 {
    match error {
        BatchesError::NotInitialized => 401,
        BatchesError::Sdk { source } => sdk_status(source),
        BatchesError::NotAsync | BatchesError::NotSync | BatchesError::Invalid { .. } => 500,
    }
}

pub(crate) fn sdk_status(error: &SdkError) -> u16 {
    if let SdkError::Status { status, .. } = error {
        return *status;
    }
    500
}

pub(crate) fn xai_status(error: &XaiBatchesError) -> u16 {
    match error {
        XaiBatchesError::MissingApiKey => 401,
        XaiBatchesError::InputFileRequired => 400,
        XaiBatchesError::Status { status, .. } => *status,
        XaiBatchesError::Limit(_)
        | XaiBatchesError::Decode { .. }
        | XaiBatchesError::Transport { .. }
        | XaiBatchesError::Path { .. }
        | XaiBatchesError::Authorization { .. }
        | XaiBatchesError::BaseUrl { .. }
        | XaiBatchesError::Encode { .. } => 500,
    }
}

// The provider entry points retrieve_batch dispatches to: the two executors this build has, and the rest named.
pub(crate) struct BatchDispatch {
    pub(crate) client: ProxiedClient,
    pub(crate) xai: xai_identity::Settings,
    pub(crate) limits: WireDecodeLimits,
}

impl BatchReadDispatch for BatchDispatch {
    type Error = DispatchFailure;

    fn retrieve(
        &self,
        call: &BatchProviderReadCall,
    ) -> impl Future<Output = Result<BatchObject, DispatchFailure>> + Send {
        let call = call.clone();
        let client = self.client.clone();
        let xai = self.xai.clone();
        let limits = self.limits;
        async move {
            match call {
                BatchProviderReadCall::OpenAiCompatible(inner) => {
                    openai_retrieve(&client, inner.as_ref()).await
                }
                BatchProviderReadCall::Xai(inner) => {
                    xai_retrieve(&client, &xai, limits, inner.as_ref()).await
                }
                BatchProviderReadCall::ProviderConfig(inner) => Err(DispatchFailure::Unsupported {
                    provider: inner.provider.name().to_owned(),
                }),
                BatchProviderReadCall::Azure(_) => Err(DispatchFailure::Unsupported {
                    provider: BatchReadProvider::Azure.name().to_owned(),
                }),
                BatchProviderReadCall::VertexAi(_) => Err(DispatchFailure::Unsupported {
                    provider: BatchReadProvider::VertexAi.name().to_owned(),
                }),
                BatchProviderReadCall::Anthropic(_) => Err(DispatchFailure::Unsupported {
                    provider: BatchReadProvider::Anthropic.name().to_owned(),
                }),
                BatchProviderReadCall::BedrockAsyncInvoke(_)
                | BatchProviderReadCall::BedrockModelInvocationJob(_) => {
                    Err(DispatchFailure::Unsupported {
                        provider: BatchReadProvider::Bedrock.name().to_owned(),
                    })
                }
            }
        }
    }
}

// OpenAIBatchesAPI.retrieve_batch
async fn openai_retrieve(
    client: &ProxiedClient,
    call: &OpenAiBatchReadCall,
) -> Result<BatchObject, DispatchFailure> {
    let query = batch_query(&call.request)?;
    let connection = Connection {
        is_async: call.is_async,
        settings: ClientSettings {
            api_key: call.api_key.clone(),
            api_base: Some(call.api_base.clone()),
            timeout: Some(call.timeout.clone()),
            max_retries: call.max_retries,
            organization: call.organization.clone(),
        },
        client: None,
    };
    openai_batches(client)
        .batch(connection, BatchOperation::Retrieve(&query))
        .await
        .map_err(DispatchFailure::from)
}

// XAIBatchesHandler.retrieve_batch
async fn xai_retrieve(
    client: &ProxiedClient,
    settings: &xai_identity::Settings,
    limits: WireDecodeLimits,
    call: &XaiBatchReadCall,
) -> Result<BatchObject, DispatchFailure> {
    let timeout = duration_of(&call.timeout)?;
    let batches = XaiBatches {
        transport: client.clone(),
        settings: settings.clone(),
        limits,
    };
    let context = CallContext {
        api_base: call.api_base.as_deref(),
        api_key: call.api_key.as_ref(),
        timeout,
    };
    batches
        .retrieve_batch(&call.batch_id, context)
        .await
        .map_err(DispatchFailure::from)
}
