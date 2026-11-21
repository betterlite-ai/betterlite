use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoSelectionError {
    #[error("no video config is installed for '{provider}'")]
    Unavailable { provider: String },
}
