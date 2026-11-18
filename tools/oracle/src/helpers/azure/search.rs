mod fixtures;
mod reports;
mod scenarios;
mod tests;

use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use scenarios::{HeaderInput, MaxResultsInput, ReplyInput, RequestInput, SettingsInput};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Request(RequestInput),
    Headers(HeaderInput),
    Citations(ReplyInput),
    MaxResults(MaxResultsInput),
    Cap(MaxResultsInput),
    Reply(ReplyInput),
    Error(ReplyInput),
    Settings(SettingsInput),
    Price(ReplyInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        reports::run(input)
    }
}
