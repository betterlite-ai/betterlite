use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_status::{VideoStatusRequest, VideoStatusResult};

pub fn execute<Metadata, Client>(
    _request: VideoStatusRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoStatusResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_status",
    }))
}

mod tests;
