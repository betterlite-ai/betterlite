use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::base::{BaseLitellmParams, OnViolation};
use super::decision_check::GuardrailChecks;
use super::decode::decode_fields;
use super::error::GuardrailParamsError;
use super::keys::PARAM_KEYS;
use super::mode::LitellmParamsMode;
use super::opaque::OpaqueParams;
use crate::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "OpaqueParams")]
pub struct LitellmParams {
    #[serde(flatten)]
    pub base: BaseLitellmParams,
    pub guardrail: String,
    pub decision_model: Option<String>,
    pub checks: Option<GuardrailChecks>,
    pub mode: LitellmParamsMode,
}

#[derive(Deserialize)]
struct ParamFields {
    guardrail: String,
    decision_model: Option<String>,
    checks: Option<GuardrailChecks>,
    mode: LitellmParamsMode,
}

fn lowercase_text(value: &mut GuardrailValue) {
    match value {
        GuardrailValue::Text(text) => *text = text.to_lowercase(),
        GuardrailValue::Array(items) => {
            for item in items {
                if let GuardrailValue::Text(text) = item {
                    *text = text.to_lowercase();
                }
            }
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_) => {}
    }
}

// LiteLLM's coerce_timeout: "" means unset, other text must read as a finite float
fn coerce_timeout(map: &mut BTreeMap<String, GuardrailValue>) -> Result<(), GuardrailParamsError> {
    let Some(GuardrailValue::Text(text)) = map.get("timeout") else {
        return Ok(());
    };
    if text.is_empty() {
        map.remove("timeout");
        return Ok(());
    }
    let seconds = betterlite_env::number::<f64>("timeout", text)
        .ok()
        .flatten()
        .filter(|seconds| seconds.is_finite())
        .ok_or(GuardrailParamsError::InvalidTimeout)?;
    map.insert("timeout".to_owned(), GuardrailValue::Decimal(seconds));
    Ok(())
}

fn normalize(map: &mut BTreeMap<String, GuardrailValue>) -> Result<(), GuardrailParamsError> {
    for key in ["guardrail", "mode"] {
        if matches!(map.get(key), None | Some(GuardrailValue::Null)) {
            return Err(GuardrailParamsError::MissingField {
                field: key.to_owned(),
            });
        }
    }
    for key in ["mode", "default_action", "on_disallowed_action"] {
        if let Some(value) = map.get_mut(key) {
            lowercase_text(value);
        }
    }
    coerce_timeout(map)?;
    if matches!(map.get("default_on"), None | Some(GuardrailValue::Null)) {
        map.insert("default_on".to_owned(), GuardrailValue::Boolean(false));
    }
    Ok(())
}

fn check_rules(base: &BaseLitellmParams, fields: &ParamFields) -> Result<(), GuardrailParamsError> {
    if let Some(violation @ (OnViolation::Block | OnViolation::Alert)) = base.fields.on_violation
        && fields.guardrail != "mcp_security"
    {
        return Err(GuardrailParamsError::ViolationNeedsMcpSecurity {
            value: violation.as_ref().to_owned(),
        });
    }
    if matches!(fields.checks, Some(GuardrailChecks::Decision(_)))
        && fields.guardrail != "decision_model"
    {
        return Err(GuardrailParamsError::ChecksNeedDecisionModel {
            guardrail: fields.guardrail.clone(),
        });
    }
    Ok(())
}

impl TryFrom<OpaqueParams> for LitellmParams {
    type Error = GuardrailParamsError;

    fn try_from(params: OpaqueParams) -> Result<Self, Self::Error> {
        let mut map = params.0;
        normalize(&mut map)?;
        let declared: BTreeMap<String, GuardrailValue> = PARAM_KEYS
            .iter()
            .filter_map(|key| {
                map.get(*key)
                    .map(|value| ((*key).to_owned(), value.clone()))
            })
            .filter(|(_key, value)| *value != GuardrailValue::Null)
            .collect();
        let bytes = serde_json::to_vec(&declared)
            .map_err(|source| GuardrailParamsError::Encode { source })?;
        let fields: ParamFields = decode_fields(&bytes)?;
        let mut base = BaseLitellmParams::from_map(map)?;
        for key in PARAM_KEYS {
            base.extra.0.remove(key);
        }
        check_rules(&base, &fields)?;
        Ok(Self {
            base,
            guardrail: fields.guardrail,
            decision_model: fields.decision_model,
            checks: fields.checks,
            mode: fields.mode,
        })
    }
}
