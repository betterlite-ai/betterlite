use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

use self::scenario::{InitScenario, LogScenario, ProjectScenario};

mod capture;
mod run;
mod scenario;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Init { scenarios: Vec<InitScenario> },
    ResolveProject { scenarios: Vec<ProjectScenario> },
    Log { scenarios: Vec<LogScenario> },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Init { scenarios } => run::init(scenarios),
            Input::ResolveProject { scenarios } => run::projects(scenarios),
            Input::Log { scenarios } => run::logs(scenarios),
        }
    }
}
