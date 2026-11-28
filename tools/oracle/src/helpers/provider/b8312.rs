mod fixtures;
mod oracle;
mod speech;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = oracle::Case;

    fn run(input: &Self::Input) -> HelperResult {
        oracle::run(input)
    }
}
