mod tests;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::errors::HarnessError;
use crate::types::Harness;

pub const STATE_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum Incompatibility {
    #[error("the harness state is unreadable")]
    Unreadable { source: serde_json::Error },
    #[error("harness state version {found} is not supported, expected {STATE_VERSION}")]
    Version { found: u32 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct State {
    pub harness: Harness,
    pub native_session_id: Option<String>,
    pub workdir: String,
    pub model: Option<String>,
    pub version: u32,
}

impl State {
    pub fn dumps(&self) -> Result<Vec<u8>, HarnessError> {
        let mut bytes = Vec::new();
        serde_json::to_writer(&mut bytes, self)
            .map_err(|source| HarnessError::StateEncode { source })?;
        Ok(bytes)
    }

    pub fn loads(data: &[u8]) -> Result<Self, HarnessError> {
        let state: Self =
            serde_json::from_slice(data).map_err(|source| HarnessError::StateIncompatible {
                source: Incompatibility::Unreadable { source },
            })?;
        if state.version != STATE_VERSION {
            return Err(HarnessError::StateIncompatible {
                source: Incompatibility::Version {
                    found: state.version,
                },
            });
        }
        Ok(state)
    }
}
