pub mod error;
mod prepare;
pub mod service;
pub mod status;

mod tests;

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::Mutex;
use std::time::Duration;

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

pub type FileResult = Result<Json, Box<SdkFailure>>;

#[derive(Clone, Copy)]
pub struct VectorFileCreateDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    pub registry: Option<&'deps dyn CredentialLookup>,
    // LiteLLM's request_timeout module constant, which REQUEST_TIMEOUT overrides
    pub request_timeout_seconds: f64,
}

#[derive(Clone, Copy)]
pub struct TransportRequest<'request> {
    pub vector_store_id: &'request str,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    pub extra_query: Option<&'request IndexMap<String, Json>>,
    pub extra_body: Option<&'request IndexMap<String, Json>>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'request str>,
    pub kwargs: &'request IndexMap<String, Json>,
}

impl TransportRequest<'_> {
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

pub struct CreateFileRequest<'request> {
    pub transport: TransportRequest<'request>,
    pub file_id: &'request str,
    pub attributes: Option<&'request Attributes>,
    pub chunking_strategy: Option<&'request IndexMap<String, Json>>,
}

pub struct ListFileRequest<'request> {
    pub transport: TransportRequest<'request>,
    pub after: Option<&'request str>,
    pub before: Option<&'request str>,
    pub filter: Option<&'request str>,
    pub limit: Option<i64>,
    pub order: Option<&'request str>,
}

// the untransformed locals and the create-only file_id that filtering must restore
struct Attachment<'request> {
    locals: Result<FileParams, serde_json::Error>,
    file_id: Option<&'request str>,
}

// everything a call needs before the operation-dependent overlays and the service are involved
struct Prepared<'request> {
    name: &'request str,
    provider: ProviderId,
    config: VectorStoreFilesConfigTag,
    params: FilesParams,
    fields: FileParams,
    timeout: Duration,
}

pub struct VectorFileCreates<'service, Service> {
    pub service: &'service Service,
    pub deps: VectorFileCreateDeps<'service>,
}

impl<Service: FileService> VectorFileCreates<'_, Service> {
    pub async fn create(&self, request: &CreateFileRequest<'_>) -> FileResult {
        let attachment = request.attachment();
        self.run(Operation::Create, &request.transport, attachment)
            .await
    }

    // acreate sets the flag itself, as LiteLLM does before calling create, so the async handler always runs
    pub async fn acreate(&self, request: &CreateFileRequest<'_>) -> FileResult {
        let mut kwargs = request.transport.kwargs.clone();
        kwargs.insert(String::from(service::ACREATE_FLAG), Json::Bool(true));
        let transport = request.transport.flagged(&kwargs);
        let attachment = request.attachment();
        self.run(Operation::Create, &transport, attachment).await
    }

    pub async fn list(&self, request: &ListFileRequest<'_>) -> FileResult {
        let attachment = request.attachment();
        self.run(Operation::List, &request.transport, attachment)
            .await
    }

    // alist sets the flag itself, as LiteLLM does before calling list, so the async handler always runs
    pub async fn alist(&self, request: &ListFileRequest<'_>) -> FileResult {
        let mut kwargs = request.transport.kwargs.clone();
        kwargs.insert(String::from(service::ALIST_FLAG), Json::Bool(true));
        let transport = request.transport.flagged(&kwargs);
        let attachment = request.attachment();
        self.run(Operation::List, &transport, attachment).await
    }

    async fn run(
        &self,
        operation: Operation,
        transport: &TransportRequest<'_>,
        attachment: Attachment<'_>,
    ) -> FileResult {
        let prepared = prepare(operation, transport, attachment, self.deps)?;
        let call = FileCall {
            operation,
            mode: mode(operation, transport.kwargs),
            provider: prepared.provider,
            config: prepared.config,
            vector_store_id: transport.vector_store_id,
            params: &prepared.params,
            fields: &prepared.fields,
            extra_headers: transport.extra_headers,
            extra_body: match operation {
                Operation::Create => transport.extra_body,
                Operation::List => None,
            },
            extra_query: match operation {
                Operation::Create => None,
                Operation::List => transport.extra_query,
            },
            timeout: prepared.timeout,
            logging: self.deps.logging,
        };
        self.service.file(&call).await.map_err(|source| {
            failure(
                &FileCallError::Service(source),
                prepared.name,
                self.deps.redactor,
            )
        })
    }
}

fn prepare<'request>(
    operation: Operation,
    transport: &TransportRequest<'request>,
    attachment: Attachment<'request>,
    deps: VectorFileCreateDeps<'_>,
) -> Result<Prepared<'request>, Box<SdkFailure>> {
    let Attachment { locals, file_id } = attachment;
    let name = ensure_provider(transport.custom_llm_provider);
    let redactor = deps.redactor;
    let entries =
        locals.map_err(|source| failure(&FileCallError::Params(source), name, redactor))?;
    let kwargs = overlay_credentials(operation, transport, deps.registry);
    let params = files_params(&kwargs).map_err(|error| failure(&error, name, redactor))?;
    let provider = resolve_provider(name).map_err(|error| failure(&error, name, redactor))?;
    let config =
        store_config(operation, provider, name).map_err(|error| failure(&error, name, redactor))?;
    let fields = merge_fields(operation.kind(), entries, &kwargs, file_id)
        .map_err(|source| failure(&FileCallError::Params(source), name, redactor))?;
    log_call(
        deps.logging,
        &kwargs,
        transport.vector_store_id,
        &fields,
        name,
    )
    .map_err(|error| failure(&error, name, redactor))?;
    let timeout = call_timeout(transport.timeout, deps.request_timeout_seconds)
        .map_err(|error| failure(&error, name, redactor))?;
    Ok(Prepared {
        name,
        provider,
        config,
        params,
        fields,
        timeout,
    })
}

impl<'request> CreateFileRequest<'request> {
    fn attachment(&self) -> Attachment<'request> {
        Attachment {
            locals: create_locals(self.file_id, self.attributes, self.chunking_strategy),
            file_id: Some(self.file_id),
        }
    }
}

impl<'request> ListFileRequest<'request> {
    fn attachment(&self) -> Attachment<'request> {
        Attachment {
            locals: list_locals(self.after, self.before, self.filter, self.limit, self.order),
            file_id: None,
        }
    }
}

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

fn resolve_provider(provider_name: &str) -> Result<ProviderId, FileCallError> {
    ProviderId::from_str(provider_name).map_err(|_invalid| FileCallError::InvalidProvider {
        provider: provider_name.to_owned(),
    })
}

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
