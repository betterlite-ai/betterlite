use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::image_generation::{ImageGenerationRequest, ImageGenerationResult};

pub fn execute<Metadata, Client>(
    _request: ImageGenerationRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<ImageGenerationResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "image_generation",
    }))
}

mod tests;
