use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// serde_json::Value is disallowed here, so both sides are compared as this typed shape
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Field {
    description: String,
    required: bool,
    #[serde(rename = "type")]
    field_type: String,
    #[serde(default)]
    options: Option<Vec<Scalar>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    default_value: Option<Option<Scalar>>,
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
    fields: Option<BTreeMap<String, Field>>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub(super) enum Scalar {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    Text(String),
}

pub(super) fn expected(text: &str) -> BTreeMap<String, Field> {
    serde_json::from_str(text).expect("the fixture is a projected form dictionary")
}

pub(super) fn projected<T: Serialize>(fields: T) -> BTreeMap<String, Field> {
    let value = serde_json::to_value(fields).expect("the projected form dictionary serializes");
    serde_json::from_value(value).expect("the projection has the shape of a form dictionary")
}
