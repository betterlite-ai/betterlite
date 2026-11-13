use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_generation::{VideoGenerationRequest, VideoGenerationResult};

pub fn execute<Metadata, Client>(
    _request: VideoGenerationRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoGenerationResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_generation",
    }))
}

mod tests;
