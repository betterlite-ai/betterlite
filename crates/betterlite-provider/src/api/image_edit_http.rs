use self::error::ImageEditHttpError;
use self::service::{
    ClientKind, ClientSource, ImageEditCall, ImageEditDeps, ImageEditOutcome, LogContext, run,
};
use crate::api::image_contracts::edit::BaseImageEditConfig;

pub mod error;
mod facts;
mod request;
pub mod service;
#[cfg(test)]
mod tests;
pub mod transport;

// LiteLLM's image_edit_handler: one POST over the shared contract, with the resolved URL handed to the finalizer
pub async fn image_edit_handler<C, S>(
    deps: &ImageEditDeps<'_, C, S>,
    call: &ImageEditCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
where
    C: BaseImageEditConfig,
    S: ClientSource,
{
    run(ClientKind::Sync, deps, call, log).await
}

pub async fn async_image_edit_handler<C, S>(
    deps: &ImageEditDeps<'_, C, S>,
    call: &ImageEditCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
where
    C: BaseImageEditConfig,
    S: ClientSource,
{
    run(ClientKind::Asynchronous, deps, call, log).await
}
