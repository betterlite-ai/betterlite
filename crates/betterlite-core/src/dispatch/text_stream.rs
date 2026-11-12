use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::{text_completion::TextCompletionRequest, text_stream::TextStreamResult};

pub fn execute<Metadata, Client>(
    _request: TextCompletionRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<TextStreamResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "text_stream",
    }))
}

mod tests;
