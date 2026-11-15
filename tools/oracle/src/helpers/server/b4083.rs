mod inputs;
mod load;
mod record;
mod support;
mod validate;

use crate::helpers::{Helper, HelperResult};

use inputs::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ValidateJob(call) => Ok(validate::validate(call)),
            Input::LoadJobs(call) => load::load(call),
            Input::RecordAttempt(call) => record::record(call),
        }
    }
}
