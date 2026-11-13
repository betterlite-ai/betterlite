use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_edit::{VideoEditRequest, VideoEditResult};

pub fn execute<Metadata, Client>(
    _request: VideoEditRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoEditResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_edit",
    }))
}

mod tests;
