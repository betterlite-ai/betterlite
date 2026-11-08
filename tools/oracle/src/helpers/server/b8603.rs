use betterlite_server::stream_keepalive::interval::is_explicit_keepalive_disable;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    values: Vec<Option<Value>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let results: Vec<Value> = input
            .values
            .iter()
            .map(|raw| Value::Bool(is_explicit_keepalive_disable(raw.as_ref())))
            .collect();
        Ok(Value::Array(results))
    }
}
