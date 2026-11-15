mod fetch;
mod run;
mod support;

use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        match input.get("function").and_then(Value::as_str) {
            Some("fetch") => fetch::fetch(input),
            Some("cache") => run::cache(input),
            Some("coordinator") => run::coordinator(input),
            _ => Err(support::value_error("unknown function".to_owned())),
        }
    }
}
