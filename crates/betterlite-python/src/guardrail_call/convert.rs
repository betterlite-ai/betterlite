use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::guardrail_records::invocation::GuardrailHookName;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::CallError;

pub(crate) mod record_input;
pub(crate) mod settings;

mod tests;

// serde_json's arbitrary_precision feature hands a JSON number to an untagged enum as this one-key map
const NUMBER_KEY: &str = "$serde_json::private::Number";

const EVENT_TYPES: [(&str, GuardrailEventHooks); 8] = [
    ("pre_call", GuardrailEventHooks::PreCall),
    ("post_call", GuardrailEventHooks::PostCall),
    ("during_call", GuardrailEventHooks::DuringCall),
    ("logging_only", GuardrailEventHooks::LoggingOnly),
    ("pre_mcp_call", GuardrailEventHooks::PreMcpCall),
    ("during_mcp_call", GuardrailEventHooks::DuringMcpCall),
    ("post_mcp_call", GuardrailEventHooks::PostMcpCall),
    (
        "realtime_input_transcription",
        GuardrailEventHooks::RealtimeInputTranscription,
    ),
];

const HOOK_NAMES: [(&str, GuardrailHookName); 6] = [
    ("async_pre_call_hook", GuardrailHookName::PreCall),
    ("async_moderation_hook", GuardrailHookName::Moderation),
    (
        "async_post_call_success_hook",
        GuardrailHookName::PostCallSuccess,
    ),
    (
        "async_post_call_streaming_hook",
        GuardrailHookName::PostCallStreaming,
    ),
    ("apply_guardrail", GuardrailHookName::ApplyGuardrail),
    ("streaming_iterator", GuardrailHookName::StreamingIterator),
];

fn find<T>(table: &[(&str, T)], name: &str) -> Option<T>
where
    T: Copy,
{
    table
        .iter()
        .find(|(wire, _hook)| *wire == name)
        .map(|(_wire, hook)| *hook)
}

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

pub(crate) fn exact_value(value: GuardrailValue) -> GuardrailValue {
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

pub(crate) fn exact_data(
    entries: BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, GuardrailValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key, exact_value(value)))
        .collect()
}

pub(crate) fn read<T>(field: &'static str, text: &str) -> Result<T, CallError>
where
    T: DeserializeOwned,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(CallError::Json { field, source }),
    }
}

pub(crate) fn write<T>(field: &'static str, value: &T) -> Result<String, CallError>
where
    T: Serialize,
{
    let encoded = serde_json::to_string(value);
    let text = encoded.map_err(|source| CallError::Json { field, source })?;
    Ok(text)
}

pub(crate) fn suppressed_from_text(text: &str) -> Result<BTreeSet<String>, CallError> {
    let names: Vec<String> = read("suppressed guardrail names", text)?;
    let mut blocked = BTreeSet::new();
    blocked.extend(names);
    Ok(blocked)
}

pub(crate) fn text_context_from_text(text: &str) -> Result<Vec<(String, String)>, CallError> {
    let raw: BTreeMap<String, GuardrailValue> = read("violation message context", text)?;
    let mut entries = Vec::with_capacity(raw.len());
    for (name, value) in raw {
        let GuardrailValue::Text(field) = value else {
            return Err(CallError::ContextValue { name });
        };
        entries.push((name, field));
    }
    Ok(entries)
}

pub(crate) fn event_type_from_name(name: &str) -> Result<GuardrailEventHooks, CallError> {
    find(&EVENT_TYPES, name).ok_or_else(|| CallError::EventType {
        name: name.to_owned(),
    })
}

pub(crate) fn hook_from_name(name: &str) -> Result<GuardrailHookName, CallError> {
    find(&HOOK_NAMES, name).ok_or_else(|| CallError::HookName {
        name: name.to_owned(),
    })
}

pub(crate) fn settings_from_text(text: &str) -> Result<GuardrailSettings, CallError> {
    read::<settings::SettingsJson>("guardrail settings", text)
        .and_then(settings::SettingsJson::into_settings)
}
