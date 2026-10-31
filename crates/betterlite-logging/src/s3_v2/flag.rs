use std::num::FpCategory;

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;
use serde_with::{As, Map, Same};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(from = "Truthiness")]
pub struct PythonBool(pub bool);

#[derive(Deserialize)]
#[serde(untagged)]
enum Truthiness {
    Bool(bool),
    Number(Number),
    Text(String),
    Sequence(Vec<IgnoredAny>),
    Mapping(#[serde(with = "As::<Map<Same, Same>>")] Vec<(String, IgnoredAny)>),
}

impl From<Truthiness> for PythonBool {
    fn from(value: Truthiness) -> Self {
        Self(match value {
            Truthiness::Bool(flag) => flag,
            Truthiness::Number(number) => number
                .as_f64()
                .is_none_or(|number_value| !matches!(number_value.classify(), FpCategory::Zero)),
            Truthiness::Text(text) => !text.is_empty(),
            Truthiness::Sequence(items) => !items.is_empty(),
            Truthiness::Mapping(fields) => !fields.is_empty(),
        })
    }
}
