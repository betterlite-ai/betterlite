use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_content::{VideoContentRequest, VideoContentResult};

pub fn execute<Metadata, Client>(
    _request: VideoContentRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoContentResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_content",
    }))
}

mod tests;
