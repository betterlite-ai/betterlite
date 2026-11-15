use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// serde_json::Value is disallowed here, so both sides are compared as this typed shape
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Field {
    description: String,
    required: bool,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    options: Option<Vec<Scalar>>,
    #[serde(default)]
    default_value: DefaultValue,
    #[serde(default)]
    dict_key_options: Option<Vec<Scalar>>,
    #[serde(default)]
    dict_value_type: Option<String>,
    #[serde(default)]
    min: Option<Scalar>,
    #[serde(default)]
    max: Option<Scalar>,
    #[serde(default)]
    step: Option<Scalar>,
    #[serde(default)]
    fields: Option<BTreeMap<String, Self>>,
}

// a key that is absent stays apart from an explicit null
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(untagged)]
pub(super) enum DefaultValue {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null,
    Value(Scalar),
}

// serde_json::Number keeps 30 and 30.0 apart and, unlike f64, survives untagged buffering under arbitrary_precision
#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub(super) enum Scalar {
    Boolean(bool),
    Number(serde_json::Number),
    Text(String),
}

pub(super) fn expected(text: &str) -> BTreeMap<String, Field> {
    serde_json::from_str(text).expect("the fixture is a projected form dictionary")
}

pub(super) fn projected<T>(fields: T) -> BTreeMap<String, Field>
where
    T: Serialize,
{
    let value = serde_json::to_value(fields).expect("the projected form dictionary serializes");
    serde_json::from_value(value).expect("the projection has the shape of a form dictionary")
}
