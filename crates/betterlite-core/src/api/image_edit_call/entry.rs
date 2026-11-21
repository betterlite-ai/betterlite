use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageResponse;
use betterlite_redact::secret::Redactor;

use super::call::{ImageEditCall, ImageEditStep, split_async_call};
use super::error::{ImageEditError, ImageEditException};
use super::logging::ImageEditLogger;

// the model and provider image_edit's except clause hands exception_type
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    pub model: Option<String>,
    pub provider: Option<String>,
}

// the seam a caller's operation fills: one preparation for both entries, ready to send or to await
pub trait ImageEditOperation {
    // asynchronous is kwargs.pop("async_call", False) is True, decided before the call reaches the operation
    fn begin(
        &self,
        call: &ImageEditCall,
        asynchronous: bool,
        logger: &mut dyn ImageEditLogger,
    ) -> Result<ImageEditStep<'_>, ImageEditError>;

    // the resolved model and provider once get_llm_provider succeeds, the caller's arguments before
    fn attribution(&self, call: &ImageEditCall) -> Attribution;
}

// the sync entry: preparation once, then a ready answer or the one coroutine a provider returned, every failure mapped by exception_type
pub async fn image_edit<O>(
    operation: &O,
    call: &ImageEditCall,
    logger: &mut dyn ImageEditLogger,
    redactor: &Redactor,
) -> Result<ImageResponse, ImageEditException>
where
    O: ImageEditOperation + Sync,
{
    let (asynchronous, kwargs) = split_async_call(&call.kwargs);
    let prepared = ImageEditCall {
        kwargs,
        ..call.clone()
    };
    let answer = match operation.begin(&prepared, asynchronous, logger) {
        Ok(ImageEditStep::Answer(answer)) => Ok(answer),
        Ok(ImageEditStep::Pending(pending)) => pending.await,
        Err(error) => Err(error),
    };
    answer.map_err(|error| {
        let attribution = operation.attribution(&prepared);
        ImageEditException::mapped(
            &error,
            attribution.model.as_deref(),
            attribution.provider.as_deref().unwrap_or_default(),
            redactor,
        )
    })
}

// forces async_call=true and preserves the call; image_edit already mapped any failure, and exception_type passes a mapped one through
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
    image_edit(operation, &asynchronous, logger, redactor).await
}
