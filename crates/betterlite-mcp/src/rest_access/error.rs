mod tests;

use std::fmt;

use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RestAccessError {
    #[error("{status}: {detail}")]
    Http { status: u16, detail: RestDetail },
    #[error("{0}")]
    Port(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RestDetail {
    Text(String),
    Error { error: String },
    ErrorMessage { error: String, message: String },
}

// ast-grep-ignore: no-spelling-impls - text renders bare and objects as JSON, by variant
impl fmt::Display for RestDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Error { .. } | Self::ErrorMessage { .. } => {
                let encoded = serde_json::to_string(self).map_err(|_source| fmt::Error)?;
                f.write_str(&encoded)
            }
        }
    }
}
