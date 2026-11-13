use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::responses::{ResponsesRequest, ResponsesResult};

#[rustfmt::skip]
pub fn execute<Metadata, Client>(
    _request: ResponsesRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<ResponsesResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "responses",
    }))
}

mod tests;
