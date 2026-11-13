use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::moderation::{ModerationRequest, ModerationResult};

pub fn execute<Metadata, Client>(
    _request: ModerationRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<ModerationResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "moderation",
    }))
}

mod tests;
