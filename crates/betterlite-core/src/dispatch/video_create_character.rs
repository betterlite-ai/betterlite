use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::video_create_character::{
    VideoCreateCharacterRequest, VideoCreateCharacterResult,
};

pub fn execute<Metadata, Client>(
    _request: VideoCreateCharacterRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<VideoCreateCharacterResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "video_create_character",
    }))
}

mod tests;
