use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{DefinitionError, RouteDefinition};

const KNOWN_FIELDS: [&str; 7] = [
    "description",
    "function_schemas",
    "llm",
    "metadata",
    "route_name",
    "similarity_threshold",
    "utterances",
];

impl RouteDefinition {
    pub fn from_value(value: &GuardrailValue) -> Result<Self, DefinitionError> {
        let GuardrailValue::Object(fields) = value else {
            return Err(DefinitionError::NotAMapping);
        };
        for key in fields.keys() {
            if !KNOWN_FIELDS.contains(&key.as_str()) {
                return Err(DefinitionError::UnknownField { field: key.clone() });
            }
        }
        let route_name = required_text(fields, "route_name")?;
        let description = optional_text(fields, "description")?.unwrap_or_default();
        let utterances = required_utterances(fields)?;
        let similarity_threshold = optional_number(fields, "similarity_threshold")?;
        Ok(Self {
            route_name,
            description,
            utterances,
            similarity_threshold,
        })
    }
}

fn required_text(
    fields: &BTreeMap<String, GuardrailValue>,
    field: &'static str,
) -> Result<String, DefinitionError> {
    match fields.get(field) {
        None => Err(DefinitionError::MissingField { field }),
        Some(GuardrailValue::Text(text)) => Ok(text.clone()),
        Some(_) => Err(DefinitionError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

fn optional_text(
    fields: &BTreeMap<String, GuardrailValue>,
    field: &'static str,
) -> Result<Option<String>, DefinitionError> {
    match fields.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(DefinitionError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

fn required_utterances(
    fields: &BTreeMap<String, GuardrailValue>,
) -> Result<Vec<String>, DefinitionError> {
    match fields.get("utterances") {
        None => Err(DefinitionError::MissingField {
            field: "utterances",
        }),
        Some(GuardrailValue::Array(items)) => items
            .iter()
            .map(|item| match item {
                GuardrailValue::Text(text) => Ok(text.clone()),
                _ => Err(DefinitionError::FieldType {
                    field: "utterances",
                    expected: "a list of strings",
                }),
            })
            .collect(),
        Some(_) => Err(DefinitionError::FieldType {
            field: "utterances",
            expected: "a list of strings",
        }),
    }
}

fn optional_number(
    fields: &BTreeMap<String, GuardrailValue>,
    field: &'static str,
) -> Result<Option<f64>, DefinitionError> {
    match fields.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(value) => number(value).map(Some).ok_or(DefinitionError::FieldType {
            field,
            expected: "a number",
        }),
    }
}

fn number(value: &GuardrailValue) -> Option<f64> {
    match value {
        GuardrailValue::Decimal(number) => Some(*number),
        GuardrailValue::SignedInteger(number) => number.to_string().parse::<f64>().ok(),
        GuardrailValue::UnsignedInteger(number) => number.to_string().parse::<f64>().ok(),
        _ => None,
    }
}
