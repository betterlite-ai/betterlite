use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod input;
mod outcome;
mod script;
mod tests;

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::CompletedTerminalError { cases }
            | Input::TransientProbeKeepsCompleted { cases }
            | Input::DetailLocations { cases }
            | Input::RecoveredRequestIdentity { cases } => outcome::status_report(cases),
            Input::ArtifactWithoutCredentials { cases } | Input::MissingVideoUrl { cases } => {
                outcome::content_report(cases)
            }
            Input::UnsupportedRequest => Ok(outcome::unsupported_report()),
        }
    }
}
