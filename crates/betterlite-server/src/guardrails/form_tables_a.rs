#![expect(
    clippy::disallowed_types,
    reason = "the form dictionary is the JSON object LiteLLM's endpoint returns, so its field values and fixture tests use serde_json values"
)]

pub mod cloud;
pub mod content;
pub mod model;
pub mod security;

use crate::guardrails::form_tables_a::model::build_model;
use crate::guardrails::provider_forms::spec::FormModel;

#[must_use]
pub fn tables_a() -> Vec<(&'static str, FormModel)> {
    security::TABLES
        .iter()
        .chain(cloud::TABLES)
        .chain(content::TABLES)
        .map(|(key, model)| (*key, build_model(model)))
        .collect()
}

mod tests;
