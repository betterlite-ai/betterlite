pub mod error;
mod prepare;
pub mod service;
pub mod status;

mod tests;

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::Mutex;

use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoreFilesConfigTag, select_store_files_config,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_file_contracts::records::{Attributes, FileParams};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::error::{FileCallError, failure};
use self::prepare::{call_timeout, create_locals, files_params, list_locals, log_call};
use self::service::{FileCall, FileService, Operation, mode};
use crate::api::vector_file_contracts::{FileParamsKind, filter_file_params};
use crate::api::vector_file_http_create_list::config::FilesParams;
use crate::api::vector_file_provider::{
    CredentialLookup, OperationKwargs, ensure_provider, prepare_registry_credentials,
};
use crate::api::vector_store_search::error::SdkFailure;

// either the attachment the provider returned or the failure B011 maps to the SDK exception it raises
pub type FileResult = Result<Json, SdkFailure>;

#[derive(Clone, Copy)]
pub struct VectorFileCreateDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    pub registry: Option<&'deps dyn CredentialLookup>,
    // LiteLLM's request_timeout module constant, which REQUEST_TIMEOUT overrides
    pub request_timeout_seconds: f64,
}

// the named arguments create and list share, with every optional one defaulting to None
#[derive(Clone, Copy)]
pub struct TransportRequest<'request> {
    pub vector_store_id: &'request str,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    // create drops this overlay before the handler; list forwards it
    pub extra_query: Option<&'request IndexMap<String, Json>>,
    // list has no body overlay; create forwards this one
    pub extra_body: Option<&'request IndexMap<String, Json>>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'request str>,
    pub kwargs: &'request IndexMap<String, Json>,
}

impl<'request> TransportRequest<'request> {
    // acreate and alist insert their flag into a copy of the kwargs, as LiteLLM does before its shared call
    const fn flagged<'flagged>(
        &'flagged self,
        kwargs: &'flagged IndexMap<String, Json>,
    ) -> TransportRequest<'flagged> {
        TransportRequest {
            vector_store_id: self.vector_store_id,
            extra_headers: self.extra_headers,
            extra_query: self.extra_query,
            extra_body: self.extra_body,
            timeout: self.timeout,
            custom_llm_provider: self.custom_llm_provider,
            kwargs,
        }
    }
}

// vector_store_file_create's named arguments
pub struct CreateFileRequest<'request> {
    pub transport: TransportRequest<'request>,
    pub file_id: &'request str,
    pub attributes: Option<&'request Attributes>,
    pub chunking_strategy: Option<&'request IndexMap<String, Json>>,
}

// vector_store_file_list's named arguments
pub struct ListFileRequest<'request> {
    pub transport: TransportRequest<'request>,
    pub after: Option<&'request str>,
    pub before: Option<&'request str>,
    pub filter: Option<&'request str>,
    pub limit: Option<i64>,
    pub order: Option<&'request str>,
}

// create/list and their async facades: the service and its collaborators are passed once
pub struct VectorFileCreates<'service, Service> {
    pub service: &'service Service,
    pub deps: VectorFileCreateDeps<'service>,
}

impl<Service: FileService> VectorFileCreates<'_, Service> {
    pub async fn create(&self, request: &CreateFileRequest<'_>) -> FileResult {
        let locals = create_locals(
            request.file_id,
            request.attributes,
            request.chunking_strategy,
        );
        self.run(
            Operation::Create,
            &request.transport,
            locals,
            Some(request.file_id),
        )
        .await
    }

    // acreate sets the flag itself, as LiteLLM does before calling create, so the async handler always runs
    pub async fn acreate(&self, request: &CreateFileRequest<'_>) -> FileResult {
        let mut kwargs = request.transport.kwargs.clone();
        kwargs.insert(String::from(service::ACREATE_FLAG), Json::Bool(true));
        let transport = request.transport.flagged(&kwargs);
        let locals = create_locals(
            request.file_id,
            request.attributes,
            request.chunking_strategy,
        );
        self.run(Operation::Create, &transport, locals, Some(request.file_id))
            .await
    }

    pub async fn list(&self, request: &ListFileRequest<'_>) -> FileResult {
        let locals = list_locals(
            request.after,
            request.before,
            request.filter,
            request.limit,
            request.order,
        );
        self.run(Operation::List, &request.transport, locals, None)
            .await
    }

    // alist sets the flag itself, as LiteLLM does before calling list, so the async handler always runs
    pub async fn alist(&self, request: &ListFileRequest<'_>) -> FileResult {
        let mut kwargs = request.transport.kwargs.clone();
        kwargs.insert(String::from(service::ALIST_FLAG), Json::Bool(true));
        let transport = request.transport.flagged(&kwargs);
        let locals = list_locals(
            request.after,
            request.before,
            request.filter,
            request.limit,
            request.order,
        );
        self.run(Operation::List, &transport, locals, None).await
    }

    async fn run(
        &self,
        operation: Operation,
        transport: &TransportRequest<'_>,
        named: Result<FileParams, serde_json::Error>,
        file_id: Option<&str>,
    ) -> FileResult {
        let provider_name = ensure_provider(transport.custom_llm_provider);
        let locals =
            named.map_err(|source| self.failed(FileCallError::Params(source), provider_name))?;
        let kwargs = overlay_credentials(operation, transport, self.deps.registry);
        let params = files_params(&kwargs).map_err(|error| self.failed(error, provider_name))?;
        let provider =
            resolve_provider(provider_name).map_err(|error| self.failed(error, provider_name))?;
        let config = store_config(operation, provider, provider_name)
            .map_err(|error| self.failed(error, provider_name))?;
        let fields = merge_fields(operation.kind(), locals, &kwargs, file_id)
            .map_err(|source| self.failed(FileCallError::Params(source), provider_name))?;
        log_call(
            self.deps.logging,
            &kwargs,
            transport.vector_store_id,
            &fields,
            provider_name,
        )
        .map_err(|error| self.failed(error, provider_name))?;
        let timeout = call_timeout(transport.timeout, self.deps.request_timeout_seconds)
            .map_err(|error| self.failed(error, provider_name))?;
        let call = FileCall {
            operation,
            mode: mode(operation, transport.kwargs),
            provider,
            config,
            vector_store_id: transport.vector_store_id,
            params: &params,
            fields: &fields,
            extra_headers: transport.extra_headers,
            extra_body: create_overlay(operation, transport.extra_body),
            extra_query: list_overlay(operation, transport.extra_query),
            timeout,
            logging: self.deps.logging,
        };
        self.service
            .file(&call)
            .await
            .map_err(|source| self.failed(FileCallError::Service(source), provider_name))
    }

    fn failed(&self, error: FileCallError, provider: &str) -> SdkFailure {
        failure(&error, provider, self.deps.redactor)
    }
}

// the async flag popped, then _prepare_registry_credentials over a copy of the caller's kwargs
fn overlay_credentials(
    operation: Operation,
    transport: &TransportRequest<'_>,
    registry: Option<&dyn CredentialLookup>,
) -> IndexMap<String, Json> {
    let mut credentials = OperationKwargs(transport.kwargs.clone());
    credentials.0.shift_remove(operation.flag());
    prepare_registry_credentials(transport.vector_store_id, &mut credentials, registry);
    credentials.0
}

// LlmProviders(custom_llm_provider), whose ValueError the SDK projection renders
fn resolve_provider(provider_name: &str) -> Result<ProviderId, FileCallError> {
    ProviderId::from_str(provider_name).map_err(|_invalid| FileCallError::InvalidProvider {
        provider: provider_name.to_owned(),
    })
}

// B010's provider-config selection, None meaning the provider has no attachment config
fn store_config(
    operation: Operation,
    provider: ProviderId,
    provider_name: &str,
) -> Result<VectorStoreFilesConfigTag, FileCallError> {
    select_store_files_config(provider).ok_or_else(|| FileCallError::Unsupported {
        operation,
        provider: provider_name.to_owned(),
    })
}

// create forwards only its body overlay; list forwards only its query overlay
const fn create_overlay(
    operation: Operation,
    extra_body: Option<&IndexMap<String, Json>>,
) -> Option<&IndexMap<String, Json>> {
    match operation {
        Operation::Create => extra_body,
        Operation::List => None,
    }
}

const fn list_overlay(
    operation: Operation,
    extra_query: Option<&IndexMap<String, Json>>,
) -> Option<&IndexMap<String, Json>> {
    match operation {
        Operation::Create => None,
        Operation::List => extra_query,
    }
}

// local_vars.update(kwargs), then B1071's non-null filter; create puts its file_id argument back after it
fn merge_fields(
    kind: FileParamsKind,
    locals: FileParams,
    kwargs: &IndexMap<String, Json>,
    file_id: Option<&str>,
) -> Result<FileParams, serde_json::Error> {
    let mut merged = locals;
    for (name, value) in kwargs {
        merged.insert(name.clone(), serde_json::to_value(value)?);
    }
    let mut fields = filter_file_params(kind, &merged);
    if let Some(attachment) = file_id {
        fields.insert(String::from("file_id"), serde_json::to_value(attachment)?);
    }
    Ok(fields)
}
