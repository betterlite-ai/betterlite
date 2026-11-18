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
        let route_name = text(
            required(field(fields, "route_name"), "route_name")?,
            "route_name",
            "a string",
        )?;
        let description = field(fields, "description")
            .map(|value| text(value, "description", "a string"))
            .transpose()?
            .unwrap_or_default();
        let utterances = match required(field(fields, "utterances"), "utterances")? {
            GuardrailValue::Array(items) => utterance_texts(items)?,
            _ => {
                return Err(DefinitionError::FieldType {
                    field: "utterances",
                    expected: "a list of strings",
                });
            }
        };
        let similarity_threshold = field(fields, "similarity_threshold")
            .map(|value| number(value, "similarity_threshold"))
            .transpose()?;
        Ok(Self {
            route_name,
            description,
            utterances,
            similarity_threshold,
        })
    }
}

// a null field reads as absent, so an explicit null description or threshold takes its default
fn field<'a>(
    fields: &'a BTreeMap<String, GuardrailValue>,
    name: &'static str,
) -> Option<&'a GuardrailValue> {
    fields
        .get(name)
        .filter(|value| !matches!(value, GuardrailValue::Null))
}

fn required<'a>(
    value: Option<&'a GuardrailValue>,
    field: &'static str,
) -> Result<&'a GuardrailValue, DefinitionError> {
    value.ok_or(DefinitionError::MissingField { field })
}

fn text(
    value: &GuardrailValue,
    field: &'static str,
    expected: &'static str,
) -> Result<String, DefinitionError> {
    match value {
        GuardrailValue::Text(text) => Ok(text.clone()),
        _ => Err(DefinitionError::FieldType { field, expected }),
    }
}

// a yaml scalar that is not a string reaches the route as an error naming the field, never as its text
fn utterance_texts(utterances: &[GuardrailValue]) -> Result<Vec<String>, DefinitionError> {
    utterances
        .iter()
        .map(|utterance| text(utterance, "utterances", "a list of strings"))
        .collect()
}

fn number(value: &GuardrailValue, field: &'static str) -> Result<f64, DefinitionError> {
    raw_number(value).ok_or(DefinitionError::FieldType {
        field,
        expected: "a number",
    })
}

fn raw_number(value: &GuardrailValue) -> Option<f64> {
    match value {
        GuardrailValue::Decimal(number) => Some(*number),
        GuardrailValue::SignedInteger(number) => number.to_string().parse::<f64>().ok(),
        GuardrailValue::UnsignedInteger(number) => number.to_string().parse::<f64>().ok(),
        _ => None,
    }
}
