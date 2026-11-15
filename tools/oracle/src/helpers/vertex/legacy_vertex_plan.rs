mod tests;

use betterlite_vertex::legacy_vertex::cache::cache_key;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) location: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(Value::String(cache_key(
            &input.model,
            input.project.as_deref(),
            input.location.as_deref(),
        )))
    }
}
