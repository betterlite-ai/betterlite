use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_get_character::{VideoGetCharacterRequest, VideoGetCharacterResult};

pub fn execute<Metadata, Client>(
    _request: VideoGetCharacterRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoGetCharacterResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_get_character",
    }))
}

mod tests;
