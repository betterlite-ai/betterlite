use tokio::runtime::Builder;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::run::execute;
use self::support::failure;
use self::types::Input;

mod fixture;
mod run;
mod support;
mod tests;
mod types;
mod views;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        execute(input, &runtime)
    }
}
