use betterlite_server::guardrails::form_tables_a::tables_a;
use betterlite_server::guardrails::provider_forms::project::{project_model, with_friendly_name};
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

// the oracle case env is empty, so LiteLLM reads DEFAULT_MAX_RECURSE_DEPTH's 100 at import
const MAX_DEPTH: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    provider: String,
}

#[derive(Debug, thiserror::Error)]
enum FormTableError {
    #[error("no config model for {0}")]
    Missing(String),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let tables = tables_a();
        let (_, table) = tables
            .iter()
            .find(|(key, _)| *key == input.provider.as_str())
            .ok_or_else(|| FormTableError::Missing(input.provider.clone()))
            .or_value_error()?;
        let mut fields = project_model(table, MAX_DEPTH).or_value_error()?;
        with_friendly_name(table, &mut fields);
        Ok(Value::Object(fields))
    }
}
