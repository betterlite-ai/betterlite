use betterlite_models::decisions::Json;

use super::error::CallError;
use super::state::LoggingHandle;
use crate::call_setup::correlation::CallOutcome;

// B2232's complete_response drain and stream_chunk_builder; None is the builder's own answer for no chunks
pub trait StreamAssembly: Send + Sync {
    fn assemble(
        &self,
        outcome: &CallOutcome,
        messages: Option<&Json>,
        logging: Option<&LoggingHandle>,
    ) -> Result<Option<Json>, CallError>;
}
