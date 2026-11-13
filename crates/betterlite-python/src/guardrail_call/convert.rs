use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::guardrail_records::invocation::GuardrailHookName;

use super::error::GuardrailCallError;

pub(crate) mod record_input;
pub(crate) mod settings;

mod tests;

// serde_json's arbitrary_precision feature hands a JSON number to an untagged enum as this one-key map
const NUMBER_KEY: &str = "$serde_json::private::Number";

fn number_from_text(text: &str) -> Option<GuardrailValue> {
    if let Ok(signed) = text.parse::<i64>() {
        return Some(GuardrailValue::SignedInteger(signed));
    }
    if let Ok(unsigned) = text.parse::<u64>() {
        return Some(GuardrailValue::UnsignedInteger(unsigned));
    }
    text.parse::<f64>()
        .ok()
        .filter(|float| float.is_finite())
        .map(GuardrailValue::Decimal)
}

fn exact_object(fields: BTreeMap<String, GuardrailValue>) -> GuardrailValue {
    if fields.len() == 1
        && let Some(GuardrailValue::Text(digits)) = fields.get(NUMBER_KEY)
        && let Some(number) = number_from_text(digits)
    {
        return number;
    }
    GuardrailValue::Object(
        fields
            .into_iter()
            .map(|(key, field)| (key, exact_value(field)))
            .collect(),
    )
}

fn exact_value(value: GuardrailValue) -> GuardrailValue {
    match value {
        GuardrailValue::Array(items) => {
            GuardrailValue::Array(items.into_iter().map(exact_value).collect())
        }
        GuardrailValue::Object(fields) => exact_object(fields),
        scalar @ (GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)) => scalar,
    }
}

pub(super) fn exact_data(
    entries: BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, GuardrailValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key, exact_value(value)))
        .collect()
}

pub(crate) fn value_from_text(text: &str) -> Result<GuardrailValue, GuardrailCallError> {
    let raw: GuardrailValue =
        serde_json::from_str(text).map_err(|source| GuardrailCallError::Json {
            field: "guardrail response",
            source,
        })?;
    Ok(exact_value(raw))
}

pub(crate) fn value_to_text(value: &GuardrailValue) -> Result<String, GuardrailCallError> {
    serde_json::to_string(value).map_err(|source| GuardrailCallError::Json {
        field: "guardrail response",
        source,
    })
}

pub(crate) fn optional_value_from_text(
    text: Option<&str>,
) -> Result<Option<GuardrailValue>, GuardrailCallError> {
    let Some(given) = text else {
        return Ok(None);
    };
    match value_from_text(given)? {
        GuardrailValue::Null => Ok(None),
        present @ (GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_)) => Ok(Some(present)),
    }
}

pub(crate) fn data_from_text(
    text: &str,
) -> Result<BTreeMap<String, GuardrailValue>, GuardrailCallError> {
    let raw: BTreeMap<String, GuardrailValue> =
        serde_json::from_str(text).map_err(|source| GuardrailCallError::Json {
            field: "request data",
            source,
        })?;
    Ok(exact_data(raw))
}

pub(crate) fn data_to_text(
    data: &BTreeMap<String, GuardrailValue>,
) -> Result<String, GuardrailCallError> {
    serde_json::to_string(data).map_err(|source| GuardrailCallError::Json {
        field: "request data",
        source,
    })
}

pub(crate) fn optional_data_from_text(
    text: Option<&str>,
) -> Result<Option<BTreeMap<String, GuardrailValue>>, GuardrailCallError> {
    let Some(given) = text else {
        return Ok(None);
    };
    let raw: Option<BTreeMap<String, GuardrailValue>> =
        serde_json::from_str(given).map_err(|source| GuardrailCallError::Json {
            field: "request data",
            source,
        })?;
    Ok(raw.map(exact_data))
}

pub(crate) fn inputs_from_text(
    text: Option<&str>,
) -> Result<Option<GuardrailApiInputs>, GuardrailCallError> {
    let Some(given) = text else {
        return Ok(None);
    };
    let inputs: Option<GuardrailApiInputs> =
        serde_json::from_str(given).map_err(|source| GuardrailCallError::Json {
            field: "guardrail inputs",
            source,
        })?;
    Ok(inputs)
}

pub(crate) fn suppressed_from_text(text: &str) -> Result<BTreeSet<String>, GuardrailCallError> {
    let names: Vec<String> =
        serde_json::from_str(text).map_err(|source| GuardrailCallError::Json {
            field: "suppressed guardrail names",
            source,
        })?;
    Ok(names.into_iter().collect())
}

pub(crate) fn text_context_from_text(
    text: &str,
) -> Result<Vec<(String, String)>, GuardrailCallError> {
    let raw: BTreeMap<String, GuardrailValue> =
        serde_json::from_str(text).map_err(|source| GuardrailCallError::Json {
            field: "violation message context",
            source,
        })?;
    let mut entries = Vec::with_capacity(raw.len());
    for (name, value) in raw {
        let GuardrailValue::Text(field) = value else {
            return Err(GuardrailCallError::ContextValue { name });
        };
        entries.push((name, field));
    }
    Ok(entries)
}

pub(crate) fn event_type_from_name(name: &str) -> Result<GuardrailEventHooks, GuardrailCallError> {
    match name {
        "pre_call" => Ok(GuardrailEventHooks::PreCall),
        "post_call" => Ok(GuardrailEventHooks::PostCall),
        "during_call" => Ok(GuardrailEventHooks::DuringCall),
        "logging_only" => Ok(GuardrailEventHooks::LoggingOnly),
        "pre_mcp_call" => Ok(GuardrailEventHooks::PreMcpCall),
        "during_mcp_call" => Ok(GuardrailEventHooks::DuringMcpCall),
        "post_mcp_call" => Ok(GuardrailEventHooks::PostMcpCall),
        "realtime_input_transcription" => Ok(GuardrailEventHooks::RealtimeInputTranscription),
        _unknown => Err(GuardrailCallError::EventType {
            name: name.to_owned(),
        }),
    }
}

pub(crate) fn hook_from_name(name: &str) -> Result<GuardrailHookName, GuardrailCallError> {
    match name {
        "async_pre_call_hook" => Ok(GuardrailHookName::PreCall),
        "async_moderation_hook" => Ok(GuardrailHookName::Moderation),
        "async_post_call_success_hook" => Ok(GuardrailHookName::PostCallSuccess),
        "async_post_call_streaming_hook" => Ok(GuardrailHookName::PostCallStreaming),
        "apply_guardrail" => Ok(GuardrailHookName::ApplyGuardrail),
        "streaming_iterator" => Ok(GuardrailHookName::StreamingIterator),
        _unknown => Err(GuardrailCallError::HookName {
            name: name.to_owned(),
        }),
    }
}

pub(crate) fn params_from_text(text: &str) -> Result<LitellmParams, GuardrailCallError> {
    serde_json::from_str(text).map_err(|source| GuardrailCallError::Json {
        field: "guardrail params",
        source,
    })
}

pub(crate) fn settings_from_text(text: &str) -> Result<GuardrailSettings, GuardrailCallError> {
    let parsed: settings::SettingsJson =
        serde_json::from_str(text).map_err(|source| GuardrailCallError::Json {
            field: "guardrail settings",
            source,
        })?;
    parsed.into_settings()
}
