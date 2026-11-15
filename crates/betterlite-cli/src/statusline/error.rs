use std::io;

use super::fetch::error::FetchError;
#[derive(Debug, thiserror::Error)]
pub enum StatuslineError {
    #[error("cannot read the hook payload from stdin: {0}")]
    Stdin(#[source] io::Error),
    #[error("cannot write the status line: {0}")]
    Stdout(#[source] io::Error),
    #[error("cannot start the async runtime: {0}")]
    Runtime(#[source] io::Error),
    #[error("cannot look up the session: {0}")]
    Fetch(#[from] FetchError),
    #[error("cannot encode the Codex message: {0}")]
    Encode(#[source] serde_json::Error),
}
