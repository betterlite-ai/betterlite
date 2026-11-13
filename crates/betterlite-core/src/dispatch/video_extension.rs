use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_extension::{VideoExtensionRequest, VideoExtensionResult};

pub fn execute<Metadata, Client>(
    _request: VideoExtensionRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoExtensionResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_extension",
    }))
}

mod tests;
