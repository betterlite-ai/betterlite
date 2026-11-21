pub mod entry;
pub mod error;
pub mod ports;
pub mod rejection;
pub mod route;

use betterlite_context::call_context::CallContext;

use crate::dispatch::error::DispatchError;
use crate::operation::llm_passthrough::{LlmPassthroughRequest, LlmPassthroughResult};

mod tests;

pub async fn execute<Metadata, Client>(
    request: LlmPassthroughRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Result<LlmPassthroughResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    match entry::execute(request).await {
        Ok(reply) => Ok(reply),
        Err(error) => Err(error.rejected()),
    }
}
