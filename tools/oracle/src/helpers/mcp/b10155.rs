use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = serde_json::Value;

    fn run(_input: &Self::Input) -> HelperResult {
        Self::not_built("b10155")
    }
}
