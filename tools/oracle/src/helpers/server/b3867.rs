mod answer;
mod inputs;
mod runtime;
mod tests;

use crate::helpers::{Helper, HelperResult};

use self::answer::{answer, failure};

pub(crate) struct Op;

impl Helper for Op {
    type Input = inputs::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(answer(input))
    }
}
