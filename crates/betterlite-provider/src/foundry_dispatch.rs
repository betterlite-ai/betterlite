use std::collections::BTreeMap;

use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::openai_base_config::CatalogCapabilities;

use crate::operation::{
    ChatOperation, OperationError, OperationRequest, OperationResult, RequestKind,
};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter;
use crate::transform_retry::send_with_retry;

pub mod agents;
pub mod chat;
pub mod claude;
pub mod command_r;
pub mod credentials;
pub mod embedding;
pub mod post_call;
pub mod route;

#[cfg(any(test, feature = "testing"))]
pub mod fixtures;

mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureAi;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureAiRequest;
pub struct AzureAiResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureAiPrepared;

impl Adapter for AzureAi {
    const NAME: &'static str = "azure_ai";

    fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        _catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        match request.kind() {
            RequestKind::Chat(operation) => prepare_chat(operation, deployment, false),
            RequestKind::Stream(operation) => prepare_chat(operation, deployment, true),
            // the embedding and agent calls run through their own composed entries, not this adapter
            RequestKind::Embedding(_) => Err(unsupported("embedding")),
            RequestKind::Other => Err(unsupported("provider")),
        }
    }

    async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        send_with_retry(client, prepared, deployment, self).await
    }

    fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        chat::decode::decode(prepared, upstream)
    }
}

fn prepare_chat(
    operation: &ChatOperation,
    deployment: &Deployment,
    stream: bool,
) -> Result<PreparedRequest, OperationError> {
    let call = chat::Call {
        model: &operation.model,
        request: &operation.request,
        extra_headers: None,
        headers: None,
        api_base: Some(deployment.api_base.as_str()),
        api_key: deployment.api_key.as_ref(),
        deployment,
        stream,
    };
    // the adapter carries its own deployment; no process-wide identity is in scope here
    let environment = BTreeMap::new();
    let settings = credentials::no_credentials();
    let globals = credentials::Globals {
        api_base: None,
        api_key: None,
        environment_api_base: None,
        environment_api_key: None,
        headers: None,
        environment: &environment,
        credentials: &settings,
        refresh_enabled: false,
        sources: &NoSources,
        token_provider: None,
    };
    Ok(chat::prepare::prepare(&call, &globals)?.prepared)
}

fn unsupported(operation: &'static str) -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from("azure_ai"),
        operation: String::from(operation),
    }
}
