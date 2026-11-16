use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod answers;
mod input;
mod secrets;
mod tests;

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AgentIds(calls) => answers::agent_ids(calls),
            Input::Params(calls) => answers::params(calls),
            Input::ProviderInfos(calls) => answers::provider_infos(calls),
            Input::CompleteUrls(calls) => answers::complete_urls(calls),
            Input::Requests(calls) => answers::requests(calls),
            Input::Preparations(calls) => answers::preparations(calls),
            Input::Validations(calls) => answers::validations(calls),
            Input::Checks(calls) => answers::checks(calls),
            Input::Errors(calls) => answers::errors(calls),
            Input::Paths(calls) => answers::paths(calls),
            Input::Flags(calls) => answers::flags(calls),
        }
    }
}
