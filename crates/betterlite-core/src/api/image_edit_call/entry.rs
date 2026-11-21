use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageResponse;
use betterlite_redact::secret::Redactor;

use super::call::{ImageEditCall, ImageEditStep, split_async_call};
use super::error::{ImageEditError, ImageEditException};
use super::logging::ImageEditLogger;

const DEFAULT_PROVIDER: &str = "openai";

// the seam a caller's operation fills: one preparation for both entries, ready to send or to await
pub trait ImageEditOperation {
    // asynchronous is kwargs.pop("async_call", False) is True, decided before the call reaches the operation
    fn begin(
        &self,
        call: &ImageEditCall,
        asynchronous: bool,
        logger: &mut dyn ImageEditLogger,
    ) -> Result<ImageEditStep<'_>, ImageEditError>;

    // get_llm_provider(model, api_base) in aimage_edit: read only to attribute an exception
    fn inferred_provider(&self, call: &ImageEditCall) -> Option<String>;
}

// the sync entry: preparation once, then a ready answer or the one coroutine a provider returned
pub async fn image_edit<O>(
    operation: &O,
    call: &ImageEditCall,
    logger: &mut dyn ImageEditLogger,
) -> Result<ImageResponse, ImageEditError>
where
    O: ImageEditOperation + Sync,
{
    let (asynchronous, kwargs) = split_async_call(&call.kwargs);
    let prepared = ImageEditCall {
        kwargs,
        ..call.clone()
    };
    match operation.begin(&prepared, asynchronous, logger)? {
        ImageEditStep::Answer(answer) => Ok(answer),
        ImageEditStep::Pending(pending) => Ok(pending.await?),
    }
}

// forces async_call=true, preserves the call, and maps a failure with the source model and provider
pub async fn aimage_edit<O>(
    operation: &O,
    call: &ImageEditCall,
    logger: &mut dyn ImageEditLogger,
    redactor: &Redactor,
) -> Result<ImageResponse, ImageEditException>
where
    O: ImageEditOperation + Sync,
{
    let mut asynchronous = call.clone();
    asynchronous
        .kwargs
        .insert(String::from("async_call"), Json::Bool(true));
    let provider = attributed_provider(operation, call);
    image_edit(operation, &asynchronous, logger)
        .await
        .map_err(|error| {
            ImageEditException::mapped(&error, call.model.as_deref(), &provider, redactor)
        })
}

// custom_llm_provider or what get_llm_provider infers from the model, defaulting to openai
fn attributed_provider<O>(operation: &O, call: &ImageEditCall) -> String
where
    O: ImageEditOperation,
{
    let inferred = match &call.custom_llm_provider {
        None => operation.inferred_provider(call),
        Some(_) => None,
    };
    call.custom_llm_provider
        .clone()
        .filter(|provider| !provider.is_empty())
        .or(inferred)
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned())
}
