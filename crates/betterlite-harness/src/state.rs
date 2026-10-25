mod tests;

use serde::{Deserialize, Serialize};

use crate::errors::HarnessError;
use crate::types::Harness;

pub const STATE_VERSION: u32 = 1;

// field order is the wire order of LiteLLM's State.dumps
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct State {
    pub harness: Harness,
    pub native_session_id: Option<String>,
    pub workdir: String,
    pub model: Option<String>,
    pub version: u32,
}

impl State {
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
