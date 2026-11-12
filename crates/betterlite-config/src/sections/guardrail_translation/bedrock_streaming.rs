use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};

#[derive(Debug, thiserror::Error)]
pub enum BedrockConfigError {
    #[error("chunk_budget_chars must be greater than 0, got {value}")]
    ChunkBudget { value: u64 },
    #[error("streaming_sampling_rate must be at least 1, got {value}")]
    SamplingRate { value: u64 },
    #[error("the Bedrock streaming extras do not decode: {source}")]
    Extras {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct ChunkBudgetChars(pub u64);

impl TryFrom<u64> for ChunkBudgetChars {
    type Error = BedrockConfigError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            return Err(BedrockConfigError::ChunkBudget { value });
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct SamplingRate(pub u64);

impl TryFrom<u64> for SamplingRate {
    type Error = BedrockConfigError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            return Err(BedrockConfigError::SamplingRate { value });
        }
        Ok(Self(value))
    }
}

const STREAMING_PARAM_KEYS: [&str; 4] = [
    "streaming_buffer_until_moderated",
    "streaming_sampling_rate",
    "streaming_end_of_stream_only",
    "streaming_buffer_release_on_scan",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BedrockGuardrailStreamParams {
    #[serde(rename = "streaming_buffer_until_moderated")]
    pub buffer_until_moderated: GuardrailFlag,
    pub streaming_sampling_rate: SamplingRate,
    pub streaming_end_of_stream_only: GuardrailFlag,
    #[serde(rename = "streaming_buffer_release_on_scan")]
    pub buffer_release_on_scan: GuardrailFlag,
}

impl Default for BedrockGuardrailStreamParams {
    fn default() -> Self {
        Self {
            buffer_until_moderated: true,
            streaming_sampling_rate: SamplingRate(5),
            streaming_end_of_stream_only: false,
            buffer_release_on_scan: false,
        }
    }
}

impl BedrockGuardrailStreamParams {
    pub fn from_extras(
        extras: Option<&BTreeMap<String, GuardrailValue>>,
    ) -> Result<Self, BedrockConfigError> {
        let Some(configured) = extras else {
            return Ok(Self::default());
        };
        let mut known = BTreeMap::new();
        for key in STREAMING_PARAM_KEYS {
            match configured.get(key) {
                Some(GuardrailValue::Null) | None => {}
                Some(value) => {
                    known.insert((*key).to_owned(), value.clone());
                }
            }
        }
        if known.is_empty() {
            return Ok(Self::default());
        }
        let encoded =
            serde_json::to_value(known).map_err(|source| BedrockConfigError::Extras { source })?;
        serde_json::from_value(encoded).map_err(|source| BedrockConfigError::Extras { source })
    }
}
