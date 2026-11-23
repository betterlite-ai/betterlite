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

// Which place Python binds the wrapper's model from, and whether a keyword model was null.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPosition {
    // args[0], which image_generation reads as its prompt
    Positional,
    // kwargs["model"] with a name; with no name the key was absent and the lookup raises KeyError
    Keyword,
    // kwargs["model"] present as null, so get_llm_provider sees None instead of a missing key
    KeywordNull,
}

// The wrapper's pre-try model lookup: a name, a present null, or the key it never had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelLookup<'model> {
    Named(&'model str),
    Null,
    Missing,
}

// The image_generation call the caller bound, with the position its first argument came from.
#[derive(Debug, Clone)]
pub struct Invocation {
    pub call: ImageGenerationCall,
    pub model_position: ModelPosition,
}

impl Invocation {
    // args[0] if len(args) > 0 else kwargs["model"]; a null value is not the absent key
    #[must_use]
    pub fn model(&self) -> ModelLookup<'_> {
        match self.model_position {
            ModelPosition::Positional => ModelLookup::Named(&self.call.prompt),
            ModelPosition::Keyword => self
                .call
                .model
                .as_deref()
                .map_or(ModelLookup::Missing, ModelLookup::Named),
            ModelPosition::KeywordNull => ModelLookup::Null,
        }
    }
}
