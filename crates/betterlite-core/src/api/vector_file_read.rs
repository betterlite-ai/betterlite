pub mod error;
pub mod service;

mod tests;

use betterlite_models::decisions::Json;

use self::error::VectorFileReadError;
use self::service::{ReadCall, ReadEntry, SdkDeps, entry_from_flag, read_attachment};
use crate::api::vector_file_http_read::service::{ClientSource, Operation};
use crate::api::vector_store_http_write::service::{ClientKind, LogContext};

// retrieve: the wrapper's own flag is popped, and only a literal true selects the asynchronous handler
pub async fn retrieve<S>(
    deps: &SdkDeps<'_, S>,
    call: &ReadCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorFileReadError>
where
    S: ClientSource,
{
    let mut kwargs = call.kwargs.clone();
    let mode = entry_from_flag(&kwargs, "aretrieve");
    kwargs.remove("aretrieve");
    let entry = ReadEntry {
        operation: Operation::Retrieve,
        mode,
        kwargs: &kwargs,
    };
    read_attachment(deps, call, log, &entry).await
}

// aretrieve: kwargs["aretrieve"] = True, so the retrieve path always takes the asynchronous handler
pub async fn aretrieve<S>(
    deps: &SdkDeps<'_, S>,
    call: &ReadCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorFileReadError>
where
    S: ClientSource,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.remove("aretrieve");
    let entry = ReadEntry {
        operation: Operation::Retrieve,
        mode: ClientKind::Asynchronous,
        kwargs: &kwargs,
    };
    read_attachment(deps, call, log, &entry).await
}

// retrieve_content: the content wrapper's own flag is the one it pops
pub async fn retrieve_content<S>(
    deps: &SdkDeps<'_, S>,
    call: &ReadCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorFileReadError>
where
    S: ClientSource,
{
    let mut kwargs = call.kwargs.clone();
    let mode = entry_from_flag(&kwargs, "aretrieve_content");
    kwargs.remove("aretrieve_content");
    let entry = ReadEntry {
        operation: Operation::Content,
        mode,
        kwargs: &kwargs,
    };
    read_attachment(deps, call, log, &entry).await
}

// aretrieve_content: kwargs["aretrieve_content"] = True, so the content path always takes the asynchronous handler
pub async fn aretrieve_content<S>(
    deps: &SdkDeps<'_, S>,
    call: &ReadCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorFileReadError>
where
    S: ClientSource,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.remove("aretrieve_content");
    let entry = ReadEntry {
        operation: Operation::Content,
        mode: ClientKind::Asynchronous,
        kwargs: &kwargs,
    };
    read_attachment(deps, call, log, &entry).await
}
