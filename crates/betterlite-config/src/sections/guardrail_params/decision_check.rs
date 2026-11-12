use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;
use serde_json::value::RawValue;

use super::decode::decode_fields;
use super::error::GuardrailParamsError;
use crate::sections::guardrail_translation::BedrockChecksConfigModel;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecisionCheckAction {
    #[default]
    Block,
    Log,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DecisionCheckWire")]
pub struct DecisionModelCheck {
    pub name: String,
    pub instructions: String,
    pub threshold: f64,
    pub action: DecisionCheckAction,
}

#[serde_inline_default]
#[derive(Deserialize)]
struct DecisionCheckWire {
    name: String,
    instructions: String,
    #[serde_inline_default(0.7)]
    threshold: f64,
    #[serde(default)]
    action: DecisionCheckAction,
}

impl TryFrom<DecisionCheckWire> for DecisionModelCheck {
    type Error = GuardrailParamsError;

    fn try_from(wire: DecisionCheckWire) -> Result<Self, Self::Error> {
        let invalid = if wire.name.is_empty() {
            Some("name")
        } else if wire.instructions.trim().is_empty() {
            Some("instructions")
        } else if !(0.0_f64..=1.0_f64).contains(&wire.threshold) {
            Some("threshold")
        } else {
            None
        };
        if let Some(field) = invalid {
            return Err(GuardrailParamsError::InvalidField {
                field: field.to_owned(),
            });
        }
        Ok(Self {
            name: wire.name,
            instructions: wire.instructions,
            threshold: wire.threshold,
            action: wire.action,
        })
    }
}

// the raw slice keeps threshold off serde's buffered untagged path, where serde_json's exact numbers fail
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum GuardrailChecks {
    Bedrock(BedrockChecksConfigModel),
    Decision(Vec<DecisionModelCheck>),
}

impl TryFrom<Box<RawValue>> for GuardrailChecks {
    type Error = GuardrailParamsError;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        if raw.get().starts_with('[') {
            return Ok(Self::Decision(decode_fields(raw.get().as_bytes())?));
        }
        Ok(Self::Bedrock(decode_fields(raw.get().as_bytes())?))
    }
}
