use betterlite_openai::threads::error::ThreadsError;
use thiserror::Error;

use crate::azure_assistant_definitions::error::DefinitionsError;

// Every thread failure comes either from preparing the SDK client or from the shared thread call.
#[derive(Debug, Error)]
pub enum ThreadMessagesError {
    #[error(transparent)]
    Client(#[from] DefinitionsError),
    #[error(transparent)]
    Threads(#[from] ThreadsError),
}
