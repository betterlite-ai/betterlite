pub mod config;
pub mod error;
pub mod service;
mod tests;

use betterlite_models::decisions::Json;

use self::config::ReadConfig;
use self::error::ReadError;
use self::service::{ClientSource, LogContext, Operation, ReadCall, ReadDeps, prepare, send};
use crate::api::vector_store_http_write::service::ClientKind;

// The four attached-file GET handlers share one read: the mode picks the client, the operation picks the transforms
pub struct VectorFileReadTransport<Source: ClientSource> {
    pub source: Source,
}

impl<Source: ClientSource> VectorFileReadTransport<Source> {
    // vector_store_file_retrieve_handler, async_vector_store_file_retrieve_handler, vector_store_file_content_handler and async_vector_store_file_content_handler; a sync handler's _is_async hand-off keeps only an async client, which is the same kind filter that picks the async handlers' pool client
    pub async fn read<C>(
        &self,
        config: &C,
        call: ReadCall<'_, Source::Client>,
        mut log: LogContext<'_, '_>,
        mode: ClientKind,
        operation: Operation,
    ) -> Result<Json, ReadError>
    where
        C: ReadConfig,
    {
        let ReadCall { target, client } = call;
        let deps = ReadDeps {
            config,
            source: &self.source,
            supplied: client,
        };
        send(prepare(&deps, mode, operation, &target, &mut log)?).await
    }
}
