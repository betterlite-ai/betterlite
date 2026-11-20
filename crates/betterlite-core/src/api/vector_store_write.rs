pub mod error;
mod prepare;
pub mod service;
mod tests;

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreExpirationPolicy;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::error::{VectorStoreWriteError, VectorStoreWriteFailure, failure};
use self::prepare::{call_timeout, log_delete, update_options};
use self::service::{DELETE_FLAG, Operation, UPDATE_FLAG, WriteCall, WriteService, mode};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::params::UpdateParams;

pub(crate) const DEFAULT_PROVIDER: &str = "openai";

// either the answer of a write or the failure B011 maps to the SDK exception it raises
pub type WriteResult = Result<Json, VectorStoreWriteFailure>;

#[derive(Clone, Copy)]
pub struct VectorStoreWriteDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    pub resolution: &'deps ResolutionContext<'deps>,
    // LiteLLM's request_timeout module constant, which REQUEST_TIMEOUT overrides
    pub request_timeout_seconds: f64,
}

// the named arguments update and delete share, with every optional one defaulting to None
#[derive(Clone, Copy)]
pub struct TransportRequest<'request> {
    pub vector_store_id: &'request str,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    // accepted to match the signature; neither write forwards a query
    pub extra_query: Option<&'request IndexMap<String, Json>>,
    pub extra_body: Option<&'request IndexMap<String, Json>>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'request str>,
    pub params: &'request WriteParams,
    pub kwargs: &'request IndexMap<String, Json>,
}

// update's declared arguments: the shared transport plus the create fields a delete ignores
pub struct VectorStoreWriteRequest<'request> {
    pub transport: TransportRequest<'request>,
    pub name: Option<&'request str>,
    pub expires_after: Option<&'request VectorStoreExpirationPolicy>,
    pub metadata: Option<&'request IndexMap<String, Json>>,
}

impl VectorStoreWriteRequest<'_> {
    // aupdate and adelete insert their flag into a copy of the kwargs, as LiteLLM does before its shared write
    const fn flagged<'flagged>(
        &'flagged self,
        kwargs: &'flagged IndexMap<String, Json>,
    ) -> VectorStoreWriteRequest<'flagged> {
        VectorStoreWriteRequest {
            transport: TransportRequest {
                vector_store_id: self.transport.vector_store_id,
                extra_headers: self.transport.extra_headers,
                extra_query: self.transport.extra_query,
                extra_body: self.transport.extra_body,
                timeout: self.transport.timeout,
                custom_llm_provider: self.transport.custom_llm_provider,
                params: self.transport.params,
                kwargs,
            },
            name: self.name,
            expires_after: self.expires_after,
            metadata: self.metadata,
        }
    }
}

// update/delete and their async façades: the service and its collaborators are passed once
pub struct VectorStoreWrites<'service, Service> {
    pub service: &'service Service,
    pub deps: VectorStoreWriteDeps<'service>,
}

impl<Service: WriteService> VectorStoreWrites<'_, Service> {
    // update: the flag still selects the async handler when it is a boolean true, as LiteLLM pops it
    pub async fn update(&self, request: &VectorStoreWriteRequest<'_>) -> WriteResult {
        let target = prepare::target(Operation::Update, &request.transport, self.deps)?;
        let options = update_options(request, self.deps, &target.name)?;
        self.send(Operation::Update, target, &options, request.transport)
            .await
    }

    // aupdate sets the flag itself, as LiteLLM does before calling update, so the async handler always runs
    pub async fn aupdate(&self, request: &VectorStoreWriteRequest<'_>) -> WriteResult {
        let mut kwargs = request.transport.kwargs.clone();
        kwargs.insert(String::from(UPDATE_FLAG), Json::Bool(true));
        self.update(&request.flagged(&kwargs)).await
    }

    pub async fn delete(&self, request: &VectorStoreWriteRequest<'_>) -> WriteResult {
        let target = prepare::target(Operation::Delete, &request.transport, self.deps)?;
        let options = log_delete(request, self.deps, &target.name)?;
        self.send(Operation::Delete, target, &options, request.transport)
            .await
    }

    // adelete sets the flag itself, as LiteLLM does before calling delete, so the async handler always runs
    pub async fn adelete(&self, request: &VectorStoreWriteRequest<'_>) -> WriteResult {
        let mut kwargs = request.transport.kwargs.clone();
        kwargs.insert(String::from(DELETE_FLAG), Json::Bool(true));
        self.delete(&request.flagged(&kwargs)).await
    }

    async fn send(
        &self,
        operation: Operation,
        target: prepare::Target,
        optional_params: &UpdateParams,
        transport: TransportRequest<'_>,
    ) -> WriteResult {
        let prepare::Target { name, id, config } = target;
        let deadline = call_timeout(transport.timeout, self.deps.request_timeout_seconds)
            .map_err(|error| failure(error, &name, self.deps.redactor))?;
        let call = WriteCall {
            operation,
            mode: mode(operation, transport.kwargs),
            provider: id,
            config,
            vector_store_id: transport.vector_store_id,
            params: transport.params,
            optional_params,
            extra_headers: transport.extra_headers,
            extra_body: transport.extra_body,
            timeout: deadline,
            logging: self.deps.logging,
        };
        self.service.write(&call).await.map_err(|error| {
            failure(
                VectorStoreWriteError::from(error),
                &name,
                self.deps.redactor,
            )
        })
    }
}
