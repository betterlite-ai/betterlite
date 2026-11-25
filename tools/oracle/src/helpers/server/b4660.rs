mod lazy;
mod refresh;
mod support;
mod tests;

use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Refresh(refresh::RefreshInput),
    LazyFetch(lazy::LazyInput),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Refresh(request) => refresh::refresh(request),
            Input::LazyFetch(request) => lazy::lazy_fetch(request),
        }
    }
}
