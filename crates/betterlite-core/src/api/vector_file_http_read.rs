pub mod config;
pub mod error;
pub mod service;
mod tests;

use betterlite_models::decisions::Json;

use crate::api::vector_store_http_write::service::ClientSource;

use self::config::ReadConfig;
use self::error::ReadError;
use self::service::{LogContext, ReadCall, ReadDeps, ReadTransport, prepare, send};

// The four attached-file GET handlers share one read: the mode picks the client, the operation picks the transforms
pub struct VectorFileReadTransport<Source: ClientSource> {
    pub source: Source,
}

impl<Source: ClientSource<Client: ReadTransport>> VectorFileReadTransport<Source> {
    // vector_store_file_retrieve_handler, async_vector_store_file_retrieve_handler, vector_store_file_content_handler and async_vector_store_file_content_handler; a sync handler's _is_async hand-off keeps only an async client, which is the same kind filter that picks the async handlers' pool client
    pub async fn read<C>(
        &self,
        config: &C,
        call: ReadCall<'_, Source::Client>,
        mut log: LogContext<'_, '_>,
    ) -> Result<Json, ReadError>
    where
        C: ReadConfig,
    {
        let ReadCall {
            target,
            client,
            mode,
            operation,
        } = call;
        let deps = ReadDeps {
            config,
            source: &self.source,
            supplied: client,
        };
        send(prepare(&deps, mode, operation, &target, &mut log)?).await
    }
}
