use std::future::Future;
use std::pin::Pin;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageResponse;

use crate::api::image_generation_call::call::ImageGenerationCall;
use crate::api::image_generation_call::error::ImageGenerationException;

// The coroutine image_generation can leave pending; the wrapper awaits it exactly once.
pub type PendingGeneration<'call> =
    Pin<Box<dyn Future<Output = Result<ImageResponse, ImageGenerationException>> + Send + 'call>>;

// What one image_generation call hands back before aimage_generation's isinstance chain runs.
pub enum GenerationReturn<'call> {
    // an ImageResponse the callee built, returned as it is
    Response(Box<ImageResponse>),
    // a dict: ImageResponse(**mapping)
    Mapping(Json),
    // a coroutine the wrapper awaits once
    Coroutine(PendingGeneration<'call>),
    // none of the three: the wrapper's ValueError
    Unexpected,
}

// Which of the two places Python binds the wrapper's model from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPosition {
    // args[0], which image_generation reads as its prompt
    Positional,
    // kwargs["model"]
    Keyword,
}

// The image_generation call the caller bound, with the position its first argument came from.
#[derive(Debug, Clone)]
pub struct Invocation {
    pub call: ImageGenerationCall,
    pub model_position: ModelPosition,
}

impl Invocation {
    // args[0] if len(args) > 0 else kwargs["model"]; a caller that passed neither saw a KeyError
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        match self.model_position {
            ModelPosition::Positional => Some(&self.call.prompt),
            ModelPosition::Keyword => self.call.model.as_deref(),
        }
    }
}
