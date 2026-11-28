use betterlite_core::api::vector_store_http_write::service::{ClientSource, WriteTransport};

use self::error::ImageEditHttpError;
use self::service::{
    ClientKind, ImageEditCall, ImageEditDeps, ImageEditOutcome, LogContext, prepare,
};
use crate::api::image_contracts::edit::BaseImageEditConfig;

pub mod asynchronous;
pub mod error;
mod facts;
mod request;
pub mod service;
mod tests;

// LiteLLM's image_edit_handler: the sync transform over the prepared request, then the tail both handlers run
pub async fn image_edit_handler<C, S>(
    deps: &ImageEditDeps<'_, C, S>,
    call: &ImageEditCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
where
    C: BaseImageEditConfig,
    S: ClientSource<Client: WriteTransport + Clone>,
{
    let prepared = prepare(ClientKind::Sync, deps, call)?;
    let transformed = deps.config.transform_image_edit_request(prepared.request());
    prepared.finish(transformed, log).await
}
