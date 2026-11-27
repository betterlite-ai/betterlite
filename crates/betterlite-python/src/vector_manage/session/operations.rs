use std::sync::Mutex;

use betterlite_core::api::vector_store_read::VectorStoreReads;
use betterlite_core::api::vector_store_read::error::VectorStoreReadFailure;
use betterlite_core::api::vector_store_read::{VectorStoreListRequest, VectorStoreReadRequest};
use betterlite_core::api::vector_store_write::VectorStoreWriteDeps;
use betterlite_core::api::vector_store_write::VectorStoreWrites;
use betterlite_core::api::vector_store_write::error::VectorStoreWriteFailure;
use betterlite_core::api::vector_store_write::service::Operation as WriteOperation;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;

use super::super::convert::{WriteWire, params, write_request};
use super::super::error::NativeFailure;
use super::{DEFAULT_PROVIDER, SessionCore};

pub(super) enum ReadRequest<'request> {
    Retrieve(VectorStoreReadRequest<'request>),
    List(VectorStoreListRequest<'request>),
}

impl SessionCore {
    // retrieve and list: B2485's provider over the session's clients, with the async façade the call's flag picks
    pub(super) async fn read(
        &self,
        request: ReadRequest<'_>,
        asynchronous: bool,
        logging: &Mutex<LoggingState>,
    ) -> Result<Json, NativeFailure> {
        let adapter = self.service();
        let deps = self.read_deps(&adapter, logging);
        let reads = VectorStoreReads {
            provider: &adapter,
            deps,
        };
        let requested = match &request {
            ReadRequest::Retrieve(retrieve) => retrieve.transport.custom_llm_provider,
            ReadRequest::List(list) => list.transport.custom_llm_provider,
        };
        let answer = match (&request, asynchronous) {
            (ReadRequest::Retrieve(retrieve), false) => reads.retrieve(retrieve).await,
            (ReadRequest::Retrieve(retrieve), true) => reads.aretrieve(retrieve).await,
            (ReadRequest::List(list), false) => reads.list(list).await,
            (ReadRequest::List(list), true) => reads.alist(list).await,
        };
        answer.map_err(|failure| read_failure(failure, requested))
    }

    // update and delete: B2402's service, with the resolution its provider selection reads
    pub(super) async fn write(
        &self,
        operation: WriteOperation,
        wire: &WriteWire,
        asynchronous: bool,
        logging: &Mutex<LoggingState>,
    ) -> Result<Json, NativeFailure> {
        let parameters = params(&wire.transport.kwargs)?;
        let request = write_request(wire, &parameters);
        let resolution = self.resolution();
        let adapter = self.service();
        let deps = VectorStoreWriteDeps {
            logging,
            redactor: &self.redactor,
            resolution: &resolution,
            request_timeout_seconds: self.write_timeout,
        };
        let writes = VectorStoreWrites {
            service: &adapter,
            deps,
        };
        let answer = match (operation, asynchronous) {
            (WriteOperation::Update, false) => writes.update(&request).await,
            (WriteOperation::Update, true) => writes.aupdate(&request).await,
            (WriteOperation::Delete, false) => writes.delete(&request).await,
            (WriteOperation::Delete, true) => writes.adelete(&request).await,
        };
        answer.map_err(|failure| {
            write_failure(failure, wire.transport.custom_llm_provider.as_deref())
        })
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
