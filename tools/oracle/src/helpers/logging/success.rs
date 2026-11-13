mod handler;
mod input;
mod output;
mod run;
mod scenario;
mod state;
mod support;

use crate::compare::helper::HelperError;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        run::run(input)
    }
}

pub(crate) fn value_error<E>(failure: &E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: failure.to_string(),
    })
}
