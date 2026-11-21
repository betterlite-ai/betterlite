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

    fn attribution(&self, call: &ImageEditCall) -> Attribution;
}

// what both entries share: the operation, the caller's logger and the redactor exception_type writes with
pub struct ImageEditEntry<'run, O> {
    pub operation: &'run O,
    pub logger: &'run mut dyn ImageEditLogger,
    pub redactor: &'run Redactor,
}

impl<O> ImageEditEntry<'_, O>
where
    O: ImageEditOperation + Sync,
{
    // the sync entry: preparation once, then a ready answer or the one coroutine a provider returned, every failure mapped by exception_type
    pub async fn image_edit(
        self,
        call: &ImageEditCall,
    ) -> Result<ImageResponse, ImageEditException> {
        let (asynchronous, kwargs) = split_async_call(&call.kwargs);
        let prepared = ImageEditCall {
            kwargs,
            ..call.clone()
        };
        let answer = match self.operation.begin(&prepared, asynchronous, self.logger) {
            Ok(ImageEditStep::Answer(answer)) => Ok(answer),
            Ok(ImageEditStep::Pending(pending)) => pending.await,
            Err(error) => Err(error),
        };
        answer.map_err(|error| {
            let attribution = self.operation.attribution(&prepared);
            ImageEditException::mapped(
                &error,
                attribution.model.as_deref(),
                attribution.provider.as_deref().unwrap_or_default(),
                self.redactor,
            )
        })
    }

    // forces async_call=true and preserves the call; image_edit already mapped any failure, and exception_type passes a mapped one through
    pub async fn aimage_edit(
        self,
        call: &ImageEditCall,
    ) -> Result<ImageResponse, ImageEditException> {
        let mut asynchronous = call.clone();
        asynchronous
            .kwargs
            .insert(String::from("async_call"), Json::Bool(true));
        self.image_edit(&asynchronous).await
    }
}
