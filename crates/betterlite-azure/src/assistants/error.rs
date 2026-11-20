use betterlite_openai::thread_runs::error::ThreadRunsError;
use thiserror::Error;

use crate::azure_assistant_definitions::error::DefinitionsError;

#[derive(Debug, Error)]
pub enum AssistantsRunError {
    #[error(transparent)]
    Client(#[from] DefinitionsError),
    #[error(transparent)]
    Runs(#[from] ThreadRunsError),
    #[error("the run request could not be encoded: {0}")]
    Encode(#[source] serde_json::Error),
}
