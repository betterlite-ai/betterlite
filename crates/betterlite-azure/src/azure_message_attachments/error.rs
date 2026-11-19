use betterlite_openai::threads::error::ThreadsError;
use thiserror::Error;

use crate::azure_assistant_definitions::error::DefinitionsError;

#[derive(Debug, Error)]
pub enum MessagesError {
    #[error(transparent)]
    Definitions(#[from] DefinitionsError),
    #[error(transparent)]
    Threads(#[from] ThreadsError),
}
