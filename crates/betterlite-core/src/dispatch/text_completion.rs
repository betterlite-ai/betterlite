use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::text_completion::{TextCompletionRequest, TextCompletionResult};

pub fn execute<Metadata, Client>(
    _request: TextCompletionRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<TextCompletionResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "text_completion",
    }))
}

mod tests;
