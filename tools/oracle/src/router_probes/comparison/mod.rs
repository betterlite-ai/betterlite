use serde_json::Value;

use crate::oracle_comparison_contracts::{self, ExpectedDifference, NumericRule};
use betterlite_oracle_runner::case::Ignore;
use betterlite_oracle_runner::compare::Difference;
use betterlite_oracle_runner::compare::value::{first_difference, remove_pointer};

pub(crate) struct Documents<'input> {
    pub(crate) reference: &'input Value,
    pub(crate) rust: &'input Value,
    pub(crate) numeric: &'input [NumericRule],
    pub(crate) expected_difference: &'input [ExpectedDifference],
    pub(crate) ignore: &'input [Ignore],
    pub(crate) self_check: bool,
    pub(crate) message: &'input str,
}

// probes, HTTP bodies and scenarios compare documents through this one contract-aware path
pub(crate) fn compare(input: &Documents<'_>) -> Option<Difference> {
    let mut reference = input.reference.clone();
    let mut rust = input.rust.clone();
    let excluded = match oracle_comparison_contracts::compare_documents(
        input.reference,
        input.rust,
        input.numeric,
        input.expected_difference,
        input.self_check,
    ) {
        Ok(pointers) => pointers,
        Err(mismatch) => {
            return Some(Difference {
                message: mismatch.message,
                pointer: mismatch.pointer,
            });
        }
    };
    for pointer in excluded
        .iter()
        .map(String::as_str)
        .chain(input.ignore.iter().map(|ignored| ignored.pointer.as_str()))
    {
        remove_pointer(&mut reference, pointer);
        remove_pointer(&mut rust, pointer);
    }
    first_difference(&reference, &rust, "").map(|pointer| Difference {
        message: input.message.to_owned(),
        pointer,
    })
}
