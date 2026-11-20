pub mod config;
pub mod error;
pub mod service;
mod tests;

use betterlite_models::decisions::Json;

use self::config::ReadConfig;
use self::error::ReadError;
use self::service::{
    ClientKind, ClientSource, LogContext, Operation, ReadCall, ReadDeps, ReadTarget, prepare, send,
};
use crate::api::vector_store_http_write::service::SuppliedClient;

// The four attached-file GET handlers share one preparation and one send; only the operation and the client kind differ
pub struct VectorFileReadTransport<Source: ClientSource> {
    pub source: Source,
}

impl<Source: ClientSource> VectorFileReadTransport<Source> {
    // vector_store_file_retrieve_handler: the _is_async hand-off comes before the sync client is picked
    pub async fn retrieve<C>(
        &self,
        config: &C,
        call: ReadCall<'_, Source::Client>,
        mut log: LogContext<'_, '_>,
        is_async: bool,
    ) -> Result<Json, ReadError>
    where
        C: ReadConfig,
    {
        let ReadCall { target, client } = call;
        if is_async {
            let forwarded = ReadCall {
                target,
                client: forwarded_client(client),
            };
            return self.async_retrieve(config, forwarded, log).await;
        }
        let deps = ReadDeps {
            config,
            source: &self.source,
            supplied: client,
        };
        run(
            &deps,
            ClientKind::Sync,
            Operation::Retrieve,
            &target,
            &mut log,
        )
        .await
    }

    // async_vector_store_file_retrieve_handler
    pub async fn async_retrieve<C>(
        &self,
        config: &C,
        call: ReadCall<'_, Source::Client>,
        mut log: LogContext<'_, '_>,
    ) -> Result<Json, ReadError>
    where
        C: ReadConfig,
    {
        let deps = ReadDeps {
            config,
            source: &self.source,
            supplied: call.client,
        };
        run(
            &deps,
            ClientKind::Asynchronous,
            Operation::Retrieve,
            &call.target,
            &mut log,
        )
        .await
    }

    // vector_store_file_content_handler: the _is_async hand-off comes before the sync client is picked
    pub async fn content<C>(
        &self,
        config: &C,
        call: ReadCall<'_, Source::Client>,
        mut log: LogContext<'_, '_>,
        is_async: bool,
    ) -> Result<Json, ReadError>
    where
        C: ReadConfig,
    {
        let ReadCall { target, client } = call;
        if is_async {
            let forwarded = ReadCall {
                target,
                client: forwarded_client(client),
            };
            return self.async_content(config, forwarded, log).await;
        }
        let deps = ReadDeps {
            config,
            source: &self.source,
            supplied: client,
        };
        run(
            &deps,
            ClientKind::Sync,
            Operation::Content,
            &target,
            &mut log,
        )
        .await
    }

    // async_vector_store_file_content_handler
    pub async fn async_content<C>(
        &self,
        config: &C,
        call: ReadCall<'_, Source::Client>,
        mut log: LogContext<'_, '_>,
    ) -> Result<Json, ReadError>
    where
        C: ReadConfig,
    {
        let deps = ReadDeps {
            config,
            source: &self.source,
            supplied: call.client,
        };
        run(
            &deps,
            ClientKind::Asynchronous,
            Operation::Content,
            &call.target,
            &mut log,
        )
        .await
    }
}

// The forwarding branch hands the async handler only an async client; the sync handler's own client is dropped there
fn forwarded_client<Client>(
    client: Option<SuppliedClient<'_, Client>>,
) -> Option<SuppliedClient<'_, Client>> {
    client.filter(|supplied| supplied.kind == ClientKind::Asynchronous)
}

async fn run<C, S>(
    deps: &ReadDeps<'_, C, S>,
    mode: ClientKind,
    operation: Operation,
    target: &ReadTarget<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, ReadError>
where
    C: ReadConfig,
    S: ClientSource,
{
    let prepared = prepare(deps, mode, operation, target, log)?;
    send(prepared).await
}
