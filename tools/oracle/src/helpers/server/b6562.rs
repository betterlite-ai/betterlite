use betterlite_server::guardrails::form_tables_b::tables_b;
use betterlite_server::guardrails::provider_forms::project::project_model;
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
    #[error("unknown form table {0}")]
    Unknown(String),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let tables = tables_b();
        let (_, model) = tables
            .iter()
            .find(|(key, _)| *key == input.provider.as_str())
            .ok_or_else(|| FormTableError::Unknown(input.provider.clone()))
            .or_value_error()?;
        let mut fields = project_model(model, MAX_DEPTH).or_value_error()?;
        let friendly_name = model
            .ui_friendly_name
            .map_or(Value::Null, |name| Value::String(name.to_owned()));
        fields.insert("ui_friendly_name".to_owned(), friendly_name);
        Ok(Value::Object(fields))
    }
}
