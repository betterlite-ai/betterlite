use std::fmt::Write as _;

use betterlite_prompts::arguments::json::{Json, Object};

use super::SchemaError;

pub(super) const CONSTRAINTS: [(&str, &str); 25] = [
    ("minItems", "minimum number of items: {}"),
    ("maxItems", "maximum number of items: {}"),
    ("uniqueItems", "all array items must be unique"),
    ("contains", "array must contain an item matching: {}"),
    ("minContains", "minimum number of matching items: {}"),
    ("maxContains", "maximum number of matching items: {}"),
    ("prefixItems", "leading items must match, in order: {}"),
    ("minimum", "minimum value: {}"),
    ("maximum", "maximum value: {}"),
    ("exclusiveMinimum", "exclusive minimum value: {}"),
    ("exclusiveMaximum", "exclusive maximum value: {}"),
    ("multipleOf", "must be a multiple of {}"),
    ("minLength", "minimum length: {}"),
    ("maxLength", "maximum length: {}"),
    ("minProperties", "minimum number of properties: {}"),
    ("maxProperties", "maximum number of properties: {}"),
    (
        "patternProperties",
        "properties whose names match each pattern must satisfy: {}",
    ),
    ("propertyNames", "property names must satisfy: {}"),
    ("dependentRequired", "dependent required properties: {}"),
    ("dependentSchemas", "dependent schemas: {}"),
    (
        "unevaluatedProperties",
        "unevaluated properties must satisfy: {}",
    ),
    ("if", "conditional (if): {}"),
    ("then", "conditional (then): {}"),
    ("else", "conditional (else): {}"),
    ("not", "must not match: {}"),
];

pub(super) fn constraint_note(schema: &Object) -> Result<Option<String>, SchemaError> {
    let mut notes = Vec::new();
    for (key, label) in CONSTRAINTS {
        if let Some(value) = schema
            .get(key)
            .filter(|value| !matches!(value, Json::Bool(false)))
        {
            let text = match value {
                Json::Null => String::from("None"),
                Json::Bool(_) => String::from("True"),
                Json::Text(text) => text.clone(),
                Json::Number(_) | Json::List(_) | Json::Object(_) => python_json(value)?,
            };
            notes.push(label.replace("{}", &text));
        }
    }
    Ok((!notes.is_empty()).then(|| format!("Note: {}.", notes.join(", "))))
}

fn python_json(value: &Json) -> Result<String, SchemaError> {
    let text = match value {
        Json::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(python_json)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        ),
        Json::Object(object) => {
            let fields = object
                .iter()
                .map(|(key, child)| Ok(format!("{}: {}", ascii_string(key)?, python_json(child)?)))
                .collect::<Result<Vec<_>, SchemaError>>()?;
            format!("{{{}}}", fields.join(", "))
        }
        Json::Text(text) => ascii_string(text)?,
        Json::Number(number) if number.is_f64() => {
            let shortest = format!(
                "{:?}",
                number.as_f64().ok_or(SchemaError::Invalid("number"))?
            );
            match shortest.split_once('e') {
                Some((mantissa, exponent)) => {
                    let (sign, digits) = exponent
                        .strip_prefix('-')
                        .map_or(("+", exponent), |digits| ("-", digits));
                    format!("{mantissa}e{sign}{digits:0>2}")
                }
                None => shortest,
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) => serde_json::to_string(value)?,
    };
    Ok(text)
}

fn ascii_string(text: &str) -> Result<String, SchemaError> {
    let encoded = serde_json::to_string(text)?;
    let mut result = String::new();
    for unit in encoded.encode_utf16() {
        if (32..127).contains(&unit) {
            result.push(char::from_u32(u32::from(unit)).ok_or(SchemaError::Invalid("string"))?);
        } else {
            write!(result, "\\u{unit:04x}")?;
        }
    }
    Ok(result)
}
