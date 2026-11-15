use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// serde_json's arbitrary_precision hands a non-integer number to a visitor as a one-entry map under this key
const NUMBER_KEY: &str = "$serde_json::private::Number";

// LiteLLM's _parse_tool_call_arguments reports one of these reasons instead of the arguments
#[derive(Debug, thiserror::Error)]
pub(super) enum ArgumentError {
    #[error("missing arguments")]
    Missing,
    #[error("arguments could not be parsed")]
    Malformed {
        #[source]
        source: serde_json::Error,
    },
    #[error("arguments must be a JSON object")]
    NotAnObject,
}

// LiteLLM reads function.arguments as either JSON text or an object and requires the result to be a mapping
pub(super) fn parse_tool_call_arguments(
    arguments: Option<&GuardrailValue>,
    tool_name: Option<&str>,
) -> Result<BTreeMap<String, GuardrailValue>, ArgumentError> {
    let Some(raw_arguments) = arguments else {
        return Err(ArgumentError::Missing);
    };
    let parsed = match restore_numbers(raw_arguments) {
        GuardrailValue::Text(source) if source.is_empty() => BTreeMap::new(),
        GuardrailValue::Text(source) => parse_json_arguments(&source, tool_name)?,
        GuardrailValue::Object(mapping) => mapping,
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_) => return Err(ArgumentError::NotAnObject),
    };
    if parsed.is_empty() {
        return Err(ArgumentError::Missing);
    }
    Ok(parsed)
}

// LiteLLM's json.loads turns a JSON number into a float; arbitrary_precision parks it in the private map instead
fn restore_numbers(value: &GuardrailValue) -> GuardrailValue {
    match value {
        GuardrailValue::Array(items) => {
            GuardrailValue::Array(items.iter().map(restore_numbers).collect())
        }
        GuardrailValue::Object(mapping) => {
            if let Some(GuardrailValue::Text(text)) = mapping.get(NUMBER_KEY)
                && mapping.len() == 1
                && let Ok(number) = text.parse::<f64>()
            {
                return GuardrailValue::Decimal(number);
            }
            GuardrailValue::Object(
                mapping
                    .iter()
                    .map(|(key, child)| (key.clone(), restore_numbers(child)))
                    .collect(),
            )
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_) => value.clone(),
    }
}

fn parse_json_arguments(
    source: &str,
    tool_name: Option<&str>,
) -> Result<BTreeMap<String, GuardrailValue>, ArgumentError> {
    let parsed: GuardrailValue = serde_json::from_str(source).map_err(|error| {
        let tool = tool_name.unwrap_or("None");
        tracing::warn!(
            "Tool Permission Guardrail: Failed to decode arguments for tool {tool}: {error}"
        );
        ArgumentError::Malformed { source: error }
    })?;
    match restore_numbers(&parsed) {
        GuardrailValue::Object(mapping) => Ok(mapping),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => {
            let tool = tool_name.unwrap_or("None");
            tracing::debug!(
                "Tool Permission Guardrail: Rejecting non-dict arguments for tool {tool}"
            );
            Err(ArgumentError::NotAnObject)
        }
    }
}
