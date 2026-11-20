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
use self::service::{Operation, WriteCall, WriteService, mode};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::params::UpdateParams;

pub(crate) const DEFAULT_PROVIDER: &str = "openai";
const UPDATE_FLAG: &str = "aupdate";
const DELETE_FLAG: &str = "adelete";

#[derive(Clone, Copy)]
pub struct VectorStoreWriteDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    pub resolution: &'deps ResolutionContext<'deps>,
    // LiteLLM's request_timeout module constant, which REQUEST_TIMEOUT overrides
    pub request_timeout_seconds: f64,
}

// the named arguments update() takes, with every optional one defaulting to None
pub struct UpdateRequest<'request> {
    pub vector_store_id: &'request str,
    pub name: Option<&'request str>,
    pub expires_after: Option<&'request VectorStoreExpirationPolicy>,
    pub metadata: Option<&'request IndexMap<String, Json>>,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    // accepted to match the signature; neither write forwards a query
    pub extra_query: Option<&'request IndexMap<String, Json>>,
    pub extra_body: Option<&'request IndexMap<String, Json>>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'request str>,
    pub params: &'request WriteParams,
    pub kwargs: &'request IndexMap<String, Json>,
}

pub struct DeleteRequest<'request> {
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

// update: the flag still selects the async handler when it is a boolean true, as LiteLLM pops it
pub async fn update<Service>(
    service: &Service,
    deps: VectorStoreWriteDeps<'_>,
    request: &UpdateRequest<'_>,
) -> Result<Json, VectorStoreWriteFailure>
where
    Service: WriteService,
{
    let target = prepare::target(Operation::Update, request.custom_llm_provider, deps)?;
    let optional_params = update_options(request, deps, &target.name)?;
    let deadline = call_timeout(request.timeout, deps.request_timeout_seconds)
        .map_err(|error| failure(error, &target.name, deps.redactor))?;
    let call = WriteCall {
        operation: Operation::Update,
        mode: mode(UPDATE_FLAG, request.kwargs),
        provider: target.id,
        config: target.config,
        vector_store_id: request.vector_store_id,
        params: request.params,
        optional_params: &optional_params,
        extra_headers: request.extra_headers,
        extra_body: request.extra_body,
        timeout: deadline,
        logging: deps.logging,
    };
    service.write(&call).await.map_err(|error| {
        failure(
            VectorStoreWriteError::from(error),
            &target.name,
            deps.redactor,
        )
    })
}

// aupdate sets the flag itself, as LiteLLM does before calling update, so the async handler always runs
pub async fn aupdate<Service>(
    service: &Service,
    deps: VectorStoreWriteDeps<'_>,
    request: &UpdateRequest<'_>,
) -> Result<Json, VectorStoreWriteFailure>
where
    Service: WriteService,
{
    let mut kwargs = request.kwargs.clone();
    kwargs.insert(String::from(UPDATE_FLAG), Json::Bool(true));
    let asynchronous = UpdateRequest {
        kwargs: &kwargs,
        ..*request
    };
    update(service, deps, &asynchronous).await
}

pub async fn delete<Service>(
    service: &Service,
    deps: VectorStoreWriteDeps<'_>,
    request: &DeleteRequest<'_>,
) -> Result<Json, VectorStoreWriteFailure>
where
    Service: WriteService,
{
    let target = prepare::target(Operation::Delete, request.custom_llm_provider, deps)?;
    log_delete(deps.logging, request, &target.name).map_err(|error| {
        failure(
            VectorStoreWriteError::Logging(error),
            &target.name,
            deps.redactor,
        )
    })?;
    let deadline = call_timeout(request.timeout, deps.request_timeout_seconds)
        .map_err(|error| failure(error, &target.name, deps.redactor))?;
    let optional_params = UpdateParams::default();
    let call = WriteCall {
        operation: Operation::Delete,
        mode: mode(DELETE_FLAG, request.kwargs),
        provider: target.id,
        config: target.config,
        vector_store_id: request.vector_store_id,
        params: request.params,
        optional_params: &optional_params,
        extra_headers: request.extra_headers,
        extra_body: request.extra_body,
        timeout: deadline,
        logging: deps.logging,
    };
    service.write(&call).await.map_err(|error| {
        failure(
            VectorStoreWriteError::from(error),
            &target.name,
            deps.redactor,
        )
    })
}

// adelete sets the flag itself, as LiteLLM does before calling delete, so the async handler always runs
pub async fn adelete<Service>(
    service: &Service,
    deps: VectorStoreWriteDeps<'_>,
    request: &DeleteRequest<'_>,
) -> Result<Json, VectorStoreWriteFailure>
where
    Service: WriteService,
{
    let mut kwargs = request.kwargs.clone();
    kwargs.insert(String::from(DELETE_FLAG), Json::Bool(true));
    let asynchronous = DeleteRequest {
        kwargs: &kwargs,
        ..*request
    };
    delete(service, deps, &asynchronous).await
}
