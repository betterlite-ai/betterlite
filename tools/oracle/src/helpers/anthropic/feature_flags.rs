use serde::Deserialize;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

mod answers;
mod calls;
mod fixture;
mod tests;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    fixture: fixture::FixtureInput,
    calls: Vec<answers::Call>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = fixture::Fixture::load(&input.fixture).or_config_error()?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        runtime.block_on(answers::run(&input.calls, &fixture))
    }
}
