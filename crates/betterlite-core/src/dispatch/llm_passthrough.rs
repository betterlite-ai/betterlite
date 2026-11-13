use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::llm_passthrough::{LlmPassthroughRequest, LlmPassthroughResult};

pub fn execute<Metadata, Client>(
    _request: LlmPassthroughRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<LlmPassthroughResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "llm_passthrough",
    }))
}

mod tests;
