use std::num::IntErrorKind;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Option<RawSettings>")]
pub struct MCPToolSearchSettings {
    pub embedding_model: Option<String>,
    pub top_k: i64,
    pub similarity_threshold: f64,
    pub core_tools: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    embedding_model: Option<String>,
    top_k: Option<i64>,
    similarity_threshold: Option<f64>,
    core_tools: Option<Vec<String>>,
}

impl Default for MCPToolSearchSettings {
    fn default() -> Self {
        Self {
            embedding_model: None,
            top_k: 5,
            similarity_threshold: 0.0,
            core_tools: Vec::new(),
        }
    }
}

impl TryFrom<Option<RawSettings>> for MCPToolSearchSettings {
    type Error = MCPToolSearchSettingsError;

    fn try_from(input: Option<RawSettings>) -> Result<Self, Self::Error> {
        let Some(raw) = input else {
            return Ok(Self::default());
        };
        let defaults = Self::default();
        let settings = Self {
            embedding_model: raw.embedding_model,
            top_k: raw.top_k.unwrap_or(defaults.top_k),
            similarity_threshold: raw
                .similarity_threshold
                .unwrap_or(defaults.similarity_threshold),
            core_tools: raw.core_tools.unwrap_or_default(),
        };
        if !(1..=100).contains(&settings.top_k) {
            return Err(MCPToolSearchSettingsError::TopKOutOfRange);
        }
        if !settings.similarity_threshold.is_finite()
            || !(0.0_f64..=1.0_f64).contains(&settings.similarity_threshold)
        {
            return Err(MCPToolSearchSettingsError::SimilarityThresholdOutOfRange);
        }
        Ok(settings)
    }
}

#[derive(Debug, Error)]
pub enum MCPToolSearchSettingsError {
    #[error("mcp_tool_search.top_k must be between 1 and 100")]
    TopKOutOfRange,
    #[error("mcp_tool_search.similarity_threshold must be between 0 and 1")]
    SimilarityThresholdOutOfRange,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum TopKValue {
    Number(serde_json::Number),
    Text(String),
    Bool(bool),
    Other(IgnoredAny),
}

#[must_use]
pub fn coerce_top_k(value: &TopKValue, default: i64) -> i64 {
    match value {
        TopKValue::Number(number) => number
            .as_i64()
            .or_else(|| {
                let decimal = number.to_string().parse::<Decimal>().ok()?;
                decimal.to_i64()
            })
            .or_else(|| number.as_f64()?.to_i64())
            .unwrap_or_else(|| {
                if number.to_string().starts_with('-') {
                    i64::MIN
                } else {
                    i64::MAX
                }
            }),
        TopKValue::Text(text) => {
            let trimmed = text.trim();
            trimmed.parse::<i64>().unwrap_or_else(|error| {
                // Parsing can report overflow before reaching an invalid trailing character.
                let digits = trimmed.trim_start_matches(['-', '+']);
                if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                    return default;
                }
                if error.kind() == &IntErrorKind::PosOverflow {
                    i64::MAX
                } else if error.kind() == &IntErrorKind::NegOverflow {
                    i64::MIN
                } else {
                    default
                }
            })
        }
        TopKValue::Bool(flag) => i64::from(*flag),
        TopKValue::Other(_) => default,
    }
}
