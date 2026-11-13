use std::fmt::Display;

use serde::Deserialize;

use crate::helpers::{HelperError, HelperResult};

mod cost;
mod credentials;
mod params;
mod reply;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Cost(cost::Steps),
    Credentials(credentials::Steps),
    Reply(reply::Steps),
    SupportedParams(params::Steps),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Cost(steps) => cost::run(steps),
            Input::Credentials(steps) => credentials::run(steps),
            Input::Reply(steps) => reply::run(steps),
            Input::SupportedParams(steps) => params::run(steps),
        }
    }
}

pub(super) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
