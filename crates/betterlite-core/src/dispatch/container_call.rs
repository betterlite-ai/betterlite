use std::future::{Ready, ready};

use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use crate::operation::container_call::{ContainerCallRequest, ContainerCallResult};

pub fn execute<Metadata, Client>(
    _request: ContainerCallRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Ready<Result<ContainerCallResult, DispatchError>> {
    ready(Err(DispatchError::NotBuilt {
        entry: "container_call",
    }))
}

mod tests;
