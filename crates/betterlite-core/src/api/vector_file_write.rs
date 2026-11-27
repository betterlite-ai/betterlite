pub mod error;
mod prepare;
pub mod service;
mod tests;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_file_contracts::records::{
    Attributes, VectorStoreFileUpdateRequest,
};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::error::{VectorFileWriteError, VectorFileWriteFailure, failure};
use self::prepare::{call_timeout, log_delete, log_update, target, update_values, write_params};
use self::service::{DELETE_FLAG, UPDATE_FLAG, WriteCall, WriteService, flag, take_mode};
use crate::api::vector_file_http_write::config::WriteParams;
use crate::api::vector_file_provider::{
    CredentialLookup, OperationKwargs, ensure_provider, prepare_registry_credentials,
};
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

// either the answer of a write or the failure B011 maps to the SDK exception it raises
pub type WriteResult = Result<Json, VectorFileWriteFailure>;

#[derive(Clone, Copy)]
pub struct VectorFileWriteDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub registry: Option<&'deps dyn CredentialLookup>,
    pub redactor: &'deps Redactor,
    // LiteLLM's request_timeout module constant, which REQUEST_TIMEOUT overrides
    pub request_timeout_seconds: f64,
}

// the named arguments both file writes share, with every optional one at its declared default
#[derive(Clone, Copy)]
pub struct FileWriteRequest<'request> {
    pub vector_store_id: &'request str,
    pub file_id: &'request str,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'request str>,
    // **kwargs: the registry's credentials land here, and GenericLiteLLMParams is built from it
    pub kwargs: &'request IndexMap<String, Json>,
}

// update's own arguments: the shared write plus the attributes it restores after the filter
#[derive(Clone, Copy)]
pub struct FileUpdateRequest<'request> {
    pub write: FileWriteRequest<'request>,
    pub attributes: Option<&'request Attributes>,
    pub extra_body: Option<&'request IndexMap<String, Json>>,
}

impl FileWriteRequest<'_> {
    // adelete inserts its flag into a copy of the kwargs, as LiteLLM does before its shared delete
    const fn flagged<'flagged>(
        &'flagged self,
        kwargs: &'flagged IndexMap<String, Json>,
    ) -> FileWriteRequest<'flagged> {
        FileWriteRequest {
            vector_store_id: self.vector_store_id,
            file_id: self.file_id,
            extra_headers: self.extra_headers,
            timeout: self.timeout,
            custom_llm_provider: self.custom_llm_provider,
            kwargs,
        }
    }
}

impl FileUpdateRequest<'_> {
    // aupdate inserts its flag into a copy of the kwargs, as LiteLLM does before its shared update
    const fn flagged<'flagged>(
        &'flagged self,
        kwargs: &'flagged IndexMap<String, Json>,
    ) -> FileUpdateRequest<'flagged> {
        FileUpdateRequest {
            write: self.write.flagged(kwargs),
            attributes: self.attributes,
            extra_body: self.extra_body,
        }
    }
}

// update/delete and their async façades: the service and its collaborators are passed once
pub struct VectorFileWrites<'service, Service> {
    pub service: &'service Service,
    pub deps: VectorFileWriteDeps<'service>,
}

struct Prepared<'call> {
    operation: Operation,
    mode: ClientKind,
    provider: ProviderId,
    config: VectorStoreFilesConfigTag,
    provider_name: String,
    vector_store_id: &'call str,
    file_id: &'call str,
    params: WriteParams,
    update_request: VectorStoreFileUpdateRequest,
    extra_headers: Option<&'call BTreeMap<String, String>>,
    extra_body: Option<&'call IndexMap<String, Json>>,
    timeout: Duration,
}

impl<Service: WriteService> VectorFileWrites<'_, Service> {
    // update: the flag in kwargs still selects the async handler when it is a boolean true, as LiteLLM pops it
    pub async fn update(&self, request: &FileUpdateRequest<'_>) -> WriteResult {
        let prepared = self.prepare(
            Operation::Update,
            &request.write,
            request.attributes,
            request.extra_body,
        )?;
        self.send(prepared).await
    }

    // aupdate sets the flag itself, as LiteLLM does before calling update, so the async handler always runs
    pub async fn aupdate(&self, request: &FileUpdateRequest<'_>) -> WriteResult {
        let mut kwargs = request.write.kwargs.clone();
        kwargs.insert(String::from(UPDATE_FLAG), Json::Bool(true));
        self.update(&request.flagged(&kwargs)).await
    }

    pub async fn delete(&self, request: &FileWriteRequest<'_>) -> WriteResult {
        let prepared = self.prepare(Operation::Delete, request, None, None)?;
        self.send(prepared).await
    }

    // adelete sets the flag itself, as LiteLLM does before calling delete, so the async handler always runs
    pub async fn adelete(&self, request: &FileWriteRequest<'_>) -> WriteResult {
        let mut kwargs = request.kwargs.clone();
        kwargs.insert(String::from(DELETE_FLAG), Json::Bool(true));
        self.delete(&request.flagged(&kwargs)).await
    }

    fn prepare<'call>(
        &self,
        operation: Operation,
        request: &'call FileWriteRequest<'_>,
        attributes: Option<&Attributes>,
        extra_body: Option<&'call IndexMap<String, Json>>,
    ) -> Result<Prepared<'call>, VectorFileWriteFailure> {
        let mut forwarded = request.kwargs.clone();
        let mode = take_mode(flag(operation), &mut forwarded);
        let provider = ensure_provider(request.custom_llm_provider);
        let deny = |error: VectorFileWriteError| failure(error, provider, self.deps.redactor);
        let mut credentials = OperationKwargs(forwarded);
        prepare_registry_credentials(
            request.vector_store_id,
            &mut credentials,
            self.deps.registry,
        );
        let merged = credentials.0;
        let params = write_params(&merged).map_err(deny)?;
        let declared = target(operation, provider, self.deps)?;
        // only the update handler has a body request; a delete's is the empty record the call carries unused
        let update_request = match operation {
            Operation::Update => update_values(attributes).map_err(deny)?,
            Operation::Delete => VectorStoreFileUpdateRequest::default(),
        };
        let entry = FileUpdateRequest {
            write: *request,
            attributes,
            extra_body,
        };
        match operation {
            Operation::Update => log_update(self.deps.logging, &entry, &merged, provider),
            Operation::Delete => log_delete(self.deps.logging, request, &merged, provider),
        }
        .map_err(deny)?;
        let timeout =
            call_timeout(request.timeout, self.deps.request_timeout_seconds).map_err(deny)?;
        Ok(Prepared {
            operation,
            mode,
            provider: declared.id,
            config: declared.config,
            provider_name: provider.to_owned(),
            vector_store_id: request.vector_store_id,
            file_id: request.file_id,
            params,
            update_request,
            extra_headers: request.extra_headers,
            extra_body,
            timeout,
        })
    }

    async fn send(&self, prepared: Prepared<'_>) -> WriteResult {
        let call = WriteCall {
            operation: prepared.operation,
            mode: prepared.mode,
            provider: prepared.provider,
            config: prepared.config,
            vector_store_id: prepared.vector_store_id,
            file_id: prepared.file_id,
            params: &prepared.params,
            update_request: &prepared.update_request,
            extra_headers: prepared.extra_headers,
            extra_body: prepared.extra_body,
            timeout: prepared.timeout,
            logging: self.deps.logging,
        };
        self.service.write(&call).await.map_err(|error| {
            failure(
                VectorFileWriteError::Write(error),
                &prepared.provider_name,
                self.deps.redactor,
            )
        })
    }
}
