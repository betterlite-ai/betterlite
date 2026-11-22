use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::router_probes::error::OperationError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuleInput {
    pub(super) keywords: Vec<String>,
    pub(super) tier: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MarkerInput {
    pub(super) open: String,
    pub(super) close: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallOut {
    pub(super) stage: String,
    pub(super) model: String,
    pub(super) inputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OverrideOut {
    pub(super) tier: String,
    pub(super) matched_keyword: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Fields {
    pub(super) plan_mode_sentinel: Option<String>,
    pub(super) plan_mode_floor: Option<String>,
    pub(super) plan_mode_floor_is_top_tier: bool,
    pub(super) applied_plan_mode_floor: Option<String>,
    pub(super) escalation_keyword: Option<String>,
    pub(super) stalled: bool,
    pub(super) housekeeping_sentinel: Option<String>,
    pub(super) housekeeping_tier: Option<String>,
    pub(super) keyword_override: Option<OverrideOut>,
    pub(super) embedding_calls: Vec<CallOut>,
}

// an absent key is None and a non-number is an error, before any key-specific conversion
fn numeric(value: Option<&Json>) -> Result<Option<&Number>, OperationError> {
    let Some(declared) = value else {
        return Ok(None);
    };
    if let Json::Number(found) = declared {
        return Ok(Some(found));
    }
    Err(invalid("the value is not a number"))
}

pub(super) fn number(value: Option<&Json>, default: f64) -> Result<f64, OperationError> {
    let Some(found) = numeric(value)? else {
        return Ok(default);
    };
    found
        .as_f64()
        .ok_or_else(|| invalid("the number is not representable"))
}

pub(super) fn count(value: Option<&Json>, default: u32) -> Result<u32, OperationError> {
    let Some(found) = numeric(value)? else {
        return Ok(default);
    };
    let Some(whole) = found.as_u64() else {
        return Err(invalid("the value is not an unsigned integer"));
    };
    u32::try_from(whole).map_err(|_source| invalid("the value does not fit an unsigned integer"))
}

pub(super) fn vector(value: &Json) -> Result<Vec<f64>, OperationError> {
    let Json::List(items) = value else {
        return Err(invalid("an embedding is not a list"));
    };
    let mut components = Vec::with_capacity(items.len());
    for item in items {
        let Json::Number(found) = item else {
            return Err(invalid("an embedding component is not a number"));
        };
        components.push(
            found
                .as_f64()
                .ok_or_else(|| invalid("an embedding component is not representable"))?,
        );
    }
    Ok(components)
}

pub(super) fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
