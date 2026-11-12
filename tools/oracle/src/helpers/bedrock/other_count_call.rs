mod input;
mod logs;
mod ops;
mod script;
mod tests;

use std::collections::BTreeMap;

use crate::helpers::{Helper, HelperResult};
use serde_json::json;

pub(crate) struct Op;

impl Helper for Op {
    type Input = BTreeMap<String, input::Calls>;

    fn run(input: &Self::Input) -> HelperResult {
        let mut calls = Vec::new();
        for (operation, group) in input {
            for call in &group.calls {
                calls.push(ops::run_call(call, operation == "counter_factory")?);
            }
        }
        Ok(json!({"calls": calls}))
    }
}
