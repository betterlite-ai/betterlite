#![expect(
    clippy::disallowed_types,
    reason = "the form dictionary is the JSON object LiteLLM's endpoint returns, so its field values and fixture tests use serde_json values"
)]

pub mod cloud;
pub mod content;
pub mod security;

use crate::guardrails::provider_forms::spec::FormModel;

#[must_use]
pub fn tables_a() -> Vec<(&'static str, FormModel)> {
    let mut tables = security::tables();
    tables.extend(cloud::tables());
    tables.extend(content::tables());
    tables
}

#[cfg(test)]
mod tests;
