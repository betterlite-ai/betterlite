use betterlite_core::api::vector_store_http_write::service::{ClientSource, WriteTransport};

use super::error::ImageEditHttpError;
use super::service::{
    ClientKind, ImageEditCall, ImageEditDeps, ImageEditOutcome, LogContext, prepare,
};
use crate::api::image_contracts::edit::BaseImageEditConfig;

// LiteLLM's async_image_edit_handler: the awaited transform over the prepared request, then the shared tail
pub async fn async_image_edit_handler<C, S>(
    deps: &ImageEditDeps<'_, C, S>,
    call: &ImageEditCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
where
    C: BaseImageEditConfig,
    S: ClientSource<Client: WriteTransport + Clone>,
{
    let prepared = prepare(ClientKind::Asynchronous, deps, call)?;
    let request = prepared.request();
    let pending = deps.config.async_transform_image_edit_request(request);
    let transformed = pending.await;
    prepared.finish(transformed, log).await
}
