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
use self::prepare::{Target, call_timeout, log_list, log_retrieve};
use self::service::{ListQuery, Operation, ReadCall, ReadProvider, VectorStoreReadConfigs, mode};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

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
pub struct VectorStoreReadRequest<'request> {
    pub transport: TransportRequest<'request>,
    pub vector_store_id: &'request str,
}

// list's four declared arguments: an absent one takes the signature's default and an explicit null is left out
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

// what one read passes on: retrieve's id, or list's four arguments
#[derive(Clone, Copy, Default)]
struct Content<'call> {
    vector_store_id: Option<&'call str>,
    query: ListQuery<'call>,
}

impl<Provider: ReadProvider> VectorStoreReads<'_, Provider> {
    // retrieve: the sync handler unless the caller's kwargs carry a boolean aretrieve
    pub async fn retrieve(&self, request: &VectorStoreReadRequest<'_>) -> ReadResult {
        let selected = mode(Operation::Retrieve, request.transport.kwargs);
        self.retrieve_with(request, selected).await
    }

    // aretrieve sets the flag itself, as LiteLLM does before calling retrieve, so the async handler always runs
    pub async fn aretrieve(&self, request: &VectorStoreReadRequest<'_>) -> ReadResult {
        self.retrieve_with(request, ClientKind::Asynchronous).await
    }

    async fn retrieve_with(
        &self,
        request: &VectorStoreReadRequest<'_>,
        selected: ClientKind,
    ) -> ReadResult {
        let target = prepare::target(Operation::Retrieve, &request.transport, &self.deps)?;
        log_retrieve(
            &self.deps,
            request.transport.kwargs,
            request.vector_store_id,
            &target.name,
        )?;
        let content = Content {
            vector_store_id: Some(request.vector_store_id),
            query: ListQuery::default(),
        };
        self.send(target, &request.transport, selected, content)
            .await
    }

    // list: the four defaults are the signature's own, so an absent argument becomes 20 and desc
    pub async fn list(&self, request: &VectorStoreListRequest<'_>) -> ReadResult {
        let selected = mode(Operation::List, request.transport.kwargs);
        self.list_with(request, selected).await
    }

    // alist sets the flag itself, as LiteLLM does before calling list, so the async handler always runs
    pub async fn alist(&self, request: &VectorStoreListRequest<'_>) -> ReadResult {
        self.list_with(request, ClientKind::Asynchronous).await
    }

    async fn list_with(
        &self,
        request: &VectorStoreListRequest<'_>,
        selected: ClientKind,
    ) -> ReadResult {
        let target = prepare::target(Operation::List, &request.transport, &self.deps)?;
        let query = effective_query(request);
        log_list(&self.deps, request.transport.kwargs, query, &target.name)?;
        let content = Content {
            vector_store_id: None,
            query,
        };
        self.send(target, &request.transport, selected, content)
            .await
    }

    async fn send(
        &self,
        target: Target<'_>,
        transport: &TransportRequest<'_>,
        selected: ClientKind,
        content: Content<'_>,
    ) -> ReadResult {
        let Target {
            operation,
            name,
            provider,
            config,
        } = target;
        let deadline = call_timeout(transport.timeout, self.deps.request_timeout_seconds)
            .map_err(|error| failure(error, &name, self.deps.redactor))?;
        let call = ReadCall {
            operation,
            mode: selected,
            provider,
            config,
            vector_store_id: content.vector_store_id,
            query: content.query,
            params: transport.params,
            extra_headers: transport.extra_headers,
            extra_body: transport.extra_body,
            timeout: deadline,
            client: self.deps.supplied.as_ref(),
            logging: self.deps.logging,
        };
        self.provider
            .read(&call)
            .await
            .map_err(|error| failure(VectorStoreReadError::Read(error), &name, self.deps.redactor))
    }
}

// the signature's defaults: limit 20 and order desc when the caller left the argument out, while an explicit null stays absent
fn effective_query<'request>(request: &VectorStoreListRequest<'request>) -> ListQuery<'request> {
    ListQuery {
        after: request.after,
        before: request.before,
        limit: request.limit.unwrap_or(Some(20)),
        order: request.order.unwrap_or(Some("desc")),
    }
}
