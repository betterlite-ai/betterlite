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
            BatchProviderCreateCall::OpenAiCompatible(inner) => self.openai_create(inner),
            BatchProviderCreateCall::Xai(inner) => self.xai_create(inner),
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

impl BatchTransport {
    // OpenAIBatchesAPI.create_batch over the shared pooled client
    pub(crate) fn openai_create(
        &self,
        call: &ProviderBatchCall,
    ) -> Result<BatchObject, BatchProviderCreateError> {
        let request = batch_create(&call.body)
            .map_err(|_source| BatchProviderCreateError::Provider { status: 400 })?;
        let connection = Connection {
            is_async: call.is_async,
            settings: ClientSettings {
                api_key: call.api_key.clone(),
                api_base: call.api_base.clone(),
                timeout: Some(Timeout::Seconds(call.timeout)),
                max_retries: call.max_retries.map(i64::from),
                organization: call.organization.clone(),
            },
            client: None,
        };
        let api = BatchesApi {
            constructor: HttpConstructor {
                transport: self.client.clone(),
                environment_key: None,
                environment_organization: None,
                environment_project: None,
            },
        };
        get_runtime()
            .block_on(api.batch(connection, BatchOperation::Create(&request)))
            .map_err(|error| BatchProviderCreateError::Provider {
                status: create_status(&error),
            })
    }

    // XAIBatchesHandler.create_batch over the shared pooled client
    pub(crate) fn xai_create(
        &self,
        call: &ProviderBatchCall,
    ) -> Result<BatchObject, BatchProviderCreateError> {
        let timeout = duration_of(&Timeout::Seconds(call.timeout))
            .map_err(|_source| BatchProviderCreateError::Provider { status: 400 })?;
        let batches = XaiBatches {
            transport: self.client.clone(),
            settings: self.xai.clone(),
            limits: self.limits,
        };
        let context = CallContext {
            api_base: call.api_base.as_deref(),
            api_key: call.api_key.as_ref(),
            timeout,
        };
        get_runtime()
            .block_on(batches.create_batch(&call.body, context))
            .map_err(|error| BatchProviderCreateError::Provider {
                status: xai_status(&error),
            })
    }
}

pub(crate) fn create_status(error: &BatchesError) -> u16 {
    match error {
        BatchesError::NotInitialized => 401,
        BatchesError::Sdk { source } => sdk_status(source),
        BatchesError::NotAsync | BatchesError::NotSync | BatchesError::Invalid { .. } => 500,
    }
}

pub(crate) fn sdk_status(error: &SdkError) -> u16 {
    match error {
        SdkError::Status { status, .. } => *status,
        SdkError::InvalidTimeout { .. }
        | SdkError::UnsupportedTimeoutPhase { .. }
        | SdkError::Transport { .. }
        | SdkError::Path { .. }
        | SdkError::HeaderName { .. }
        | SdkError::Encode { .. }
        | SdkError::Body { .. }
        | SdkError::BaseUrl { .. } => 500,
    }
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
    let api = BatchesApi {
        constructor: HttpConstructor {
            transport: client.clone(),
            environment_key: None,
            environment_organization: None,
            environment_project: None,
        },
    };
    api.batch(connection, BatchOperation::Retrieve(&query))
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
