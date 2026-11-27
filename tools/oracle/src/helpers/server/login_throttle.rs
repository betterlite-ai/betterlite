use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::login_throttle::support::{OracleInput, run_oracle};

pub(crate) struct Op;

impl Helper for Op {
    type Input = OracleInput;

    fn run(input: &Self::Input) -> HelperResult {
        let output = run_oracle(input).or_value_error()?;
        Ok(serde_json::to_value(output)?)
    }
}
