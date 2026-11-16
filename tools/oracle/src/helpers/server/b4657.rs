mod fetch;
mod run;
mod support;

use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Fetch(fetch::FetchInput),
    Cache(run::CacheInput),
    Coordinator,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Fetch(request) => fetch::fetch(request),
            Input::Cache(request) => run::cache(request),
            Input::Coordinator => run::coordinator(),
        }
    }
}
