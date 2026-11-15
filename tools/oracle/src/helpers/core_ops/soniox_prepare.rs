mod answers;
mod input;
mod tests;

use serde_json::json;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .calls
            .iter()
            .map(answers::answer)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": answers }))
    }
}
