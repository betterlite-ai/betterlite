mod chat;
mod placement;
mod route;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ChatInjections { probes: Vec<chat::Probe> },
    MessageInjections { probes: Vec<placement::Probe> },
    MessagesRoute { probes: Vec<route::Probe> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ChatInjections { probes } => chat::run(probes),
            Input::MessageInjections { probes } => {
                let results: Vec<Value> = probes.iter().map(placement::result).collect();
                Ok(json!({ "probes": results }))
            }
            Input::MessagesRoute { probes } => {
                let results: Vec<Value> = probes.iter().map(route::result).collect();
                Ok(json!({ "probes": results }))
            }
        }
    }
}
