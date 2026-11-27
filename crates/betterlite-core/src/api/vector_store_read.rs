pub mod error;
mod prepare;
pub mod service;
mod tests;

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::error::{VectorStoreReadError, VectorStoreReadFailure, failure};
use self::prepare::{call_timeout, list_options, retrieve_options};
use self::service::{
    LIST_FLAG, ListQuery, Operation, RETRIEVE_FLAG, ReadCall, ReadProvider, VectorStoreReadConfigs,
    mode,
};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::service::SuppliedClient;

pub(crate) const DEFAULT_PROVIDER: &str = "openai";

// either the answer of a read or the failure B011 maps to the SDK exception it raises
pub type ReadResult = Result<Json, VectorStoreReadFailure>;

pub struct VectorStoreReadDeps<'deps, Client> {
    pub configs: &'deps dyn VectorStoreReadConfigs,
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    // a client the caller built for this one call, used when its kind matches the handler's
    pub supplied: Option<SuppliedClient<'deps, Client>>,
    // LiteLLM's request_timeout module constant, which REQUEST_TIMEOUT overrides
    pub request_timeout_seconds: f64,
}

// the kwargs and overrides retrieve and list share, plus the base and key GenericLiteLLMParams read; extra_query is accepted and neither read forwards it
#[derive(Clone, Copy)]
pub struct TransportRequest<'request> {
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    pub extra_query: Option<&'request IndexMap<String, Json>>,
    pub extra_body: Option<&'request IndexMap<String, Json>>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'request str>,
    pub params: &'request WriteParams,
    pub kwargs: &'request IndexMap<String, Json>,
}

// retrieve's one named argument and the shared transport
#[derive(Clone, Copy)]
pub struct VectorStoreReadRequest<'request> {
    pub transport: TransportRequest<'request>,
    pub vector_store_id: &'request str,
}

// list's four declared arguments: an absent one takes the signature's default and an explicit null is left out
#[derive(Clone, Copy)]
pub struct VectorStoreListRequest<'request> {
    pub transport: TransportRequest<'request>,
    pub after: Option<&'request str>,
    pub before: Option<&'request str>,
    pub limit: Option<Option<i64>>,
    pub order: Option<Option<&'request str>>,
}

// retrieve/list and their async façades: the provider's read service and its collaborators are passed once
pub struct VectorStoreReads<'service, Provider: ReadProvider> {
    pub provider: &'service Provider,
    pub deps: VectorStoreReadDeps<'service, Provider::Client>,
}

// what one read adds to the shared target: retrieve's id, or list's four arguments with their defaults applied
#[derive(Clone, Copy)]
pub(crate) struct Content<'call> {
    vector_store_id: Option<&'call str>,
    query: ListQuery<'call>,
}

impl<Provider: ReadProvider> VectorStoreReads<'_, Provider> {
    // retrieve: the sync handler unless the caller's kwargs carry a boolean aretrieve
    pub async fn retrieve(&self, request: &VectorStoreReadRequest<'_>) -> ReadResult {
        let target = prepare::target(Operation::Retrieve, &request.transport, &self.deps)?;
        let content = retrieve_options(&self.deps, request, &target.name)?;
        self.send(target, request.transport, content).await
    }

    // aretrieve sets the flag itself, as LiteLLM does before calling retrieve, so the async handler always runs
    pub async fn aretrieve(&self, request: &VectorStoreReadRequest<'_>) -> ReadResult {
        let mut kwargs = request.transport.kwargs.clone();
        kwargs.insert(String::from(RETRIEVE_FLAG), Json::Bool(true));
        let mut flagged: VectorStoreReadRequest<'_> = *request;
        flagged.transport.kwargs = &kwargs;
        self.retrieve(&flagged).await
    }

    // list: the sync handler unless the caller's kwargs carry a boolean alist
    pub async fn list(&self, request: &VectorStoreListRequest<'_>) -> ReadResult {
        let target = prepare::target(Operation::List, &request.transport, &self.deps)?;
        let content = list_options(&self.deps, request, &target.name)?;
        self.send(target, request.transport, content).await
    }

    // alist sets the flag itself, as LiteLLM does before calling list, so the async handler always runs
    pub async fn alist(&self, request: &VectorStoreListRequest<'_>) -> ReadResult {
        let mut kwargs = request.transport.kwargs.clone();
        kwargs.insert(String::from(LIST_FLAG), Json::Bool(true));
        let mut flagged: VectorStoreListRequest<'_> = *request;
        flagged.transport.kwargs = &kwargs;
        self.list(&flagged).await
    }

    // the provider's read service sends it: the target names where, the kwargs' flag picks the handler
    async fn send(
        &self,
        target: prepare::Target<'_>,
        transport: TransportRequest<'_>,
        content: Content<'_>,
    ) -> ReadResult {
        let deadline = call_timeout(transport.timeout, self.deps.request_timeout_seconds)
            .map_err(|error| failure(error, &target.name, self.deps.redactor))?;
        let call = ReadCall {
            operation: target.operation,
            mode: mode(target.operation, transport.kwargs),
            provider: target.provider,
            config: target.config,
            vector_store_id: content.vector_store_id,
            query: content.query,
            params: transport.params,
            extra_headers: transport.extra_headers,
            extra_body: transport.extra_body,
            timeout: deadline,
            client: self.deps.supplied.as_ref(),
            logging: self.deps.logging,
        };
        self.provider.read(&call).await.map_err(|error| {
            failure(
                VectorStoreReadError::Read(error),
                &target.name,
                self.deps.redactor,
            )
        })
    }
}
