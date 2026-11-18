use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod credentials;
mod failure;
mod http;
mod input;
mod ops;
mod runtime;
mod signed;
mod tests;
mod transform;
mod transport;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}
