use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::CustomCodeGuardrail;
use super::error::CustomCodeApplyError;
use super::error::CustomCodeGuardrailError;
use super::inputs::CustomCodeInputs;
use super::records;
use super::runtime::input_type_name;
use crate::generic_guardrail_contracts::GuardrailInputType;

pub(super) const DEFAULT_BLOCK_REASON: &str = "Blocked by custom code guardrail";
pub(super) const DEFAULT_FLAG_REASON: &str = "Flagged by custom code guardrail";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Action {
    Allow,
    Block,
    Flag,
    Modify,
    Unknown(String),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Moment {
    pub input_type: GuardrailInputType,
    pub started: Duration,
}

// LiteLLM defaults a missing action to allow and treats any other unknown action as allow too.
#[must_use]
pub(super) fn action(fields: &BTreeMap<String, GuardrailValue>) -> Action {
    match fields.get("action") {
        None => Action::Allow,
        Some(GuardrailValue::Text(text)) => match text.as_str() {
            "allow" => Action::Allow,
            "block" => Action::Block,
            "flag" => Action::Flag,
            "modify" => Action::Modify,
            _ => Action::Unknown(text.clone()),
        },
        Some(other) => Action::Unknown(serde_json::to_string(other).unwrap_or_default()),
    }
}

pub(super) fn dispatch(
    guardrail: &CustomCodeGuardrail,
    fields: &BTreeMap<String, GuardrailValue>,
    inputs: CustomCodeInputs,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    moment: Moment,
) -> Result<CustomCodeInputs, CustomCodeApplyError> {
    match action(fields) {
        Action::Allow => {
            tracing::debug!(
                "Custom code guardrail '{}': Allowing {}",
                guardrail.guardrail_name,
                input_type_name(moment.input_type)
            );
            Ok(inputs)
        }
        Action::Block => Err(block(&guardrail.guardrail_name, fields, moment.input_type)?),
        Action::Flag => {
            flag(guardrail, fields, request_data, moment)?;
            Ok(inputs)
        }
        Action::Modify => Ok(modify(guardrail, fields, moment.input_type, inputs)?),
        Action::Unknown(action) => {
            tracing::warn!(
                "Custom code guardrail '{}': Unknown action '{action}'. Treating as allow.",
                guardrail.guardrail_name
            );
            Ok(inputs)
        }
    }
}

pub(super) fn block(
    guardrail_name: &str,
    fields: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Result<CustomCodeApplyError, CustomCodeGuardrailError> {
    let reason = text_field(fields, "reason", DEFAULT_BLOCK_REASON)?;
    let detection_info = object_field(fields, "detection_info")?;
    tracing::info!(
        "Custom code guardrail '{guardrail_name}': Blocking {} - {reason}",
        input_type_name(input_type)
    );
    // LiteLLM raises a passthrough exception before the model call and an HTTP 400 for a response.
    if input_type == GuardrailInputType::Request {
        return Ok(CustomCodeApplyError::Passthrough {
            message: reason,
            detection_info,
        });
    }
    Ok(CustomCodeApplyError::Blocked {
        reason,
        guardrail: guardrail_name.to_owned(),
        detection_info,
    })
}

pub(super) fn flag(
    guardrail: &CustomCodeGuardrail,
    fields: &BTreeMap<String, GuardrailValue>,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    moment: Moment,
) -> Result<(), CustomCodeGuardrailError> {
    let reason = text_field(fields, "reason", DEFAULT_FLAG_REASON)?;
    let metadata = object_field(fields, "metadata")?;
    tracing::info!(
        "Custom code guardrail '{}': Flagging {} - {reason}",
        guardrail.guardrail_name,
        input_type_name(moment.input_type)
    );
    let record = records::FlagRecord {
        guardrail_name: guardrail.guardrail_name.clone(),
        reason,
        input_type: moment.input_type,
        metadata,
        started: moment.started,
        ended: (guardrail.clock)(),
    };
    records::append_flag_record(request_data, &record);
    Ok(())
}

// B6670 derives the same Python type names from the JSON kind for the test route's warning.
#[must_use]
pub(super) const fn value_type_name(value: &GuardrailValue) -> &'static str {
    match value {
        GuardrailValue::Null => "NoneType",
        GuardrailValue::Boolean(_) => "bool",
        GuardrailValue::SignedInteger(_) | GuardrailValue::UnsignedInteger(_) => "int",
        GuardrailValue::Decimal(_) => "float",
        GuardrailValue::Text(_) => "str",
        GuardrailValue::Array(_) => "list",
        GuardrailValue::Object(_) => "dict",
    }
}

pub(super) fn modify(
    guardrail: &CustomCodeGuardrail,
    fields: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    mut inputs: CustomCodeInputs,
) -> Result<CustomCodeInputs, CustomCodeGuardrailError> {
    tracing::debug!(
        "Custom code guardrail '{}': Modifying {}",
        guardrail.guardrail_name,
        input_type_name(input_type)
    );
    if let Some(value) = fields
        .get("texts")
        .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        inputs.texts = Some(text_list("texts", value)?);
    }
    if let Some(value) = fields
        .get("images")
        .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        inputs.images = Some(text_list("images", value)?);
    }
    if let Some(value) = fields
        .get("tool_calls")
        .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        let GuardrailValue::Array(calls) = value else {
            return Err(invalid_field("tool_calls", "a list"));
        };
        inputs.tool_calls = Some(calls.clone());
    }
    Ok(inputs)
}

fn invalid_field(key: &str, expected: &str) -> CustomCodeGuardrailError {
    CustomCodeGuardrailError::ExecutionMessage {
        message: format!("custom code guardrail result field '{key}' must be {expected}"),
    }
}

fn text_field(
    fields: &BTreeMap<String, GuardrailValue>,
    key: &str,
    default: &str,
) -> Result<String, CustomCodeGuardrailError> {
    match fields.get(key) {
        None | Some(GuardrailValue::Null) => Ok(default.to_owned()),
        Some(GuardrailValue::Text(text)) => Ok(text.clone()),
        Some(_) => Err(invalid_field(key, "text")),
    }
}

fn object_field(
    fields: &BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Result<BTreeMap<String, GuardrailValue>, CustomCodeGuardrailError> {
    let present = fields
        .get(key)
        .filter(|value| !matches!(value, GuardrailValue::Null));
    let Some(value) = present else {
        return Ok(BTreeMap::new());
    };
    let GuardrailValue::Object(object) = value else {
        return Err(invalid_field(key, "an object"));
    };
    Ok(object.clone())
}

fn text_list(field: &str, value: &GuardrailValue) -> Result<Vec<String>, CustomCodeGuardrailError> {
    let GuardrailValue::Array(items) = value else {
        return Err(invalid_field(field, "a list of texts"));
    };
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        let GuardrailValue::Text(text) = item else {
            return Err(invalid_field(field, "a list of texts"));
        };
        texts.push(text.clone());
    }
    Ok(texts)
}
