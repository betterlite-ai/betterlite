use std::sync::Mutex;

use betterlite_core::api::vector_store_read::VectorStoreReads;
use betterlite_core::api::vector_store_read::error::VectorStoreReadFailure;
use betterlite_core::api::vector_store_write::error::VectorStoreWriteFailure;
use betterlite_core::api::vector_store_write::service::Operation as WriteOperation;
use betterlite_core::api::vector_store_write::{
    VectorStoreWriteDeps, VectorStoreWriteRequest, VectorStoreWrites,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;

use super::super::convert::params;
use super::super::convert::requests::{
    delete_request, list_request, retrieve_request, update_request,
};
use super::super::convert::{DeleteWire, ListWire, RetrieveWire, UpdateWire};
use super::super::error::NativeFailure;
use super::super::service::{ReadService, WriteAdapter};
use super::{DEFAULT_PROVIDER, SessionCore};

impl SessionCore {
    pub(super) async fn retrieve(
        &self,
        wire: &RetrieveWire,
        asynchronous: bool,
        logging: &Mutex<LoggingState>,
    ) -> Result<Json, NativeFailure> {
        let transport = params(&wire.kwargs)?;
        let request = retrieve_request(wire, &transport);
        let configs = self.configs();
        let service = ReadService {
            source: &self.source,
            redactor: &self.redactor,
            clock: self.clock.as_ref(),
        };
        let deps = self.read_deps(&configs, logging);
        let reads = VectorStoreReads {
            provider: &service,
            deps,
        };
        let answer = if asynchronous {
            reads.aretrieve(&request).await
        } else {
            reads.retrieve(&request).await
        };
        answer.map_err(|failure| read_failure(failure, wire.custom_llm_provider.as_deref()))
    }

    pub(super) async fn list(
        &self,
        wire: &ListWire,
        asynchronous: bool,
        logging: &Mutex<LoggingState>,
    ) -> Result<Json, NativeFailure> {
        let transport = params(&wire.kwargs)?;
        let request = list_request(wire, &transport);
        let configs = self.configs();
        let service = ReadService {
            source: &self.source,
            redactor: &self.redactor,
            clock: self.clock.as_ref(),
        };
        let deps = self.read_deps(&configs, logging);
        let reads = VectorStoreReads {
            provider: &service,
            deps,
        };
        let answer = if asynchronous {
            reads.alist(&request).await
        } else {
            reads.list(&request).await
        };
        answer.map_err(|failure| read_failure(failure, wire.custom_llm_provider.as_deref()))
    }

    pub(super) async fn update(
        &self,
        wire: &UpdateWire,
        asynchronous: bool,
        logging: &Mutex<LoggingState>,
    ) -> Result<Json, NativeFailure> {
        let transport = params(&wire.kwargs)?;
        let request = update_request(wire, &transport);
        let requested = wire.custom_llm_provider.as_deref();
        self.write(
            WriteOperation::Update,
            &request,
            asynchronous,
            requested,
            logging,
        )
        .await
    }

    pub(super) async fn delete(
        &self,
        wire: &DeleteWire,
        asynchronous: bool,
        logging: &Mutex<LoggingState>,
    ) -> Result<Json, NativeFailure> {
        let transport = params(&wire.kwargs)?;
        let request = delete_request(wire, &transport);
        let requested = wire.custom_llm_provider.as_deref();
        self.write(
            WriteOperation::Delete,
            &request,
            asynchronous,
            requested,
            logging,
        )
        .await
    }

    // update and delete: the write core's service, with the resolution its provider selection reads
    async fn write(
        &self,
        operation: WriteOperation,
        request: &VectorStoreWriteRequest<'_>,
        asynchronous: bool,
        requested: Option<&str>,
        logging: &Mutex<LoggingState>,
    ) -> Result<Json, NativeFailure> {
        let resolution = self.resolution();
        let service = WriteAdapter {
            credentials: &self.credentials,
            globals: &self.globals,
            env_api_key: &self.env_api_key,
            source: &self.source,
            redactor: &self.redactor,
            clock: self.clock.as_ref(),
        };
        let deps = VectorStoreWriteDeps {
            logging,
            redactor: &self.redactor,
            resolution: &resolution,
            request_timeout_seconds: self.write_timeout,
        };
        let writes = VectorStoreWrites {
            service: &service,
            deps,
        };
        let answer = match (operation, asynchronous) {
            (WriteOperation::Update, false) => writes.update(request).await,
            (WriteOperation::Update, true) => writes.aupdate(request).await,
            (WriteOperation::Delete, false) => writes.delete(request).await,
            (WriteOperation::Delete, true) => writes.adelete(request).await,
        };
        answer.map_err(|failure| write_failure(failure, requested))
    }
}

// the provider a failure reports: the caller's declaration, or openai when it named none
fn read_failure(failure: VectorStoreReadFailure, requested: Option<&str>) -> NativeFailure {
    NativeFailure::Read {
        failure,
        provider: requested.unwrap_or(DEFAULT_PROVIDER).to_owned(),
    }
}

fn write_failure(failure: VectorStoreWriteFailure, requested: Option<&str>) -> NativeFailure {
    NativeFailure::Write {
        failure,
        provider: requested.unwrap_or(DEFAULT_PROVIDER).to_owned(),
    }
}
