use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

use steps::Steps;

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
        Ok(match input {
            Input::PasswordGrantForm(steps) | Input::DefaultTenant(steps) => {
                Value::Array(steps.steps.iter().map(report::grant_step).collect())
            }
            Input::InvalidGrant(steps) | Input::PasswordScopeDecoration(steps) => {
                Value::Array(steps.steps.iter().map(report::attempt_step).collect())
            }
            Input::TokenResponse(steps) => Value::Array(
                steps
                    .steps
                    .iter()
                    .map(report::token_response_step)
                    .collect(),
            ),
            Input::UsernameDebugWarning(steps) | Input::UsernameConstructorFailure(steps) => {
                Value::Array(steps.steps.iter().map(report::construction_step).collect())
            }
        })
    }
}
