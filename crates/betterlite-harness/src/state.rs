mod tests;

use serde::{Deserialize, Serialize};

use crate::errors::HarnessError;
use crate::types::Harness;

pub const STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct State {
    pub harness: Harness,
    pub native_session_id: Option<String>,
    pub workdir: String,
    pub model: Option<String>,
    pub version: u32,
}

impl State {
    #[must_use]
    pub fn dumps(&self) -> Vec<u8> {
        match serde_json::to_vec(self) {
            Ok(bytes) => bytes,
            Err(error) => {
                drop(error);
                Vec::new()
            }
        }
    }

    pub fn loads(data: &[u8]) -> Result<Self, HarnessError> {
        let state: Self = serde_json::from_slice(data)
            .map_err(|source| HarnessError::StateIncompatible { source })?;
        if state.version != STATE_VERSION {
            return Err(HarnessError::StateVersion {
                found: state.version,
            });
        }
        Ok(state)
    }
}
