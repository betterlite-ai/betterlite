use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_list::{VideoListRequest, VideoListResult};

pub fn execute<Metadata, Client>(
    _request: VideoListRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoListResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_list",
    }))
}

mod tests;
