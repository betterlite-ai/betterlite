use thiserror::Error;

use crate::gemini_live_client::error::LiveClientError;
use crate::gemini_live_setup::error::LiveSetupError;

#[derive(Debug, Error)]
pub enum VertexLiveError {
    #[error(transparent)]
    Setup(#[from] LiveSetupError),
    #[error(transparent)]
    Client(#[from] LiveClientError),
}
