use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

use steps::{Step, Steps};

mod fixture;
mod report;
mod steps;
mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    PasswordGrantForm(Steps),
    DefaultTenant(Steps),
    InvalidGrant(Steps),
    TokenResponse(Steps),
    UsernameDebugWarning(Steps),
    UsernameConstructorFailure(Steps),
    PasswordScopeDecoration(Steps),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let (steps, convert): (&Steps, fn(&Step) -> Value) = match input {
            Input::PasswordGrantForm(steps) | Input::DefaultTenant(steps) => {
                (steps, report::grant_step)
            }
            Input::InvalidGrant(steps)
            | Input::TokenResponse(steps)
            | Input::PasswordScopeDecoration(steps) => (steps, report::attempt_step),
            Input::UsernameDebugWarning(steps) | Input::UsernameConstructorFailure(steps) => {
                (steps, report::construction_step)
            }
        };
        Ok(Value::Array(steps.steps.iter().map(convert).collect()))
    }
}
