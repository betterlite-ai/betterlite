use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_remix::{VideoRemixRequest, VideoRemixResult};

pub fn execute<Metadata, Client>(
    _request: VideoRemixRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoRemixResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_remix",
    }))
}

mod tests;
