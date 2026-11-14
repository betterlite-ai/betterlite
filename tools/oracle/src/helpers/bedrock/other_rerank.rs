use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

mod driver;
mod fixtures;
mod input;
mod tests;

use driver::observe;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.probes.len());
        for probe in &input.probes {
            results.push(observe(probe, input.function)?);
        }
        Ok(Value::Array(results))
    }
}
