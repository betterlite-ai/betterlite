use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::image_edit::{ImageEditRequest, ImageEditResult};

pub fn execute<Metadata, Client>(
    _request: ImageEditRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<ImageEditResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "image_edit",
    }))
}

mod tests;
