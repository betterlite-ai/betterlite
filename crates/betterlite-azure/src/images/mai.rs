use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::logging::ImageGenerationLogger;
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use indexmap::IndexMap;

use super::error::ClassifiedError;

// B554 owns the MAI reply (usage normalization, defaults, the size from width and height), and a provider leaf
// cannot import betterlite-foundry, so the caller supplies it and this crate only picks when to run it
pub struct MaiReply<'reply> {
    pub body: &'reply [u8],
    pub status_code: u16,
    // LiteLLM hands the same data dict as request_data, optional_params and litellm_params
    pub data: &'reply IndexMap<String, Json>,
}

pub trait MaiProjection {
    fn project(
        &self,
        reply: MaiReply<'_>,
        logger: &mut dyn ImageGenerationLogger,
    ) -> Result<ImageGenerationReply, ClassifiedError>;
}
