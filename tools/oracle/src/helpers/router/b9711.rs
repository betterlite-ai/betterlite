mod guardrail;
mod settings;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Settings(settings::SettingsInput),
    Guardrail(guardrail::GuardrailInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Settings(settings) => settings::run(settings),
            Input::Guardrail(guardrail) => guardrail::run(guardrail),
        }
    }
}
