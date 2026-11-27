use std::sync::{Arc, Mutex};

use betterlite_config::sections::router_compaction::ContextCompactionConfig;
use betterlite_router::compaction_state::{
    CompactionState, SharedCompactionState, compaction_pending,
};
use betterlite_router::router_predicates::JsonMap;
use serde::{Deserialize, Serialize};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Absent,
    Unarmed,
    Armed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    state: State,
    payload: JsonMap,
}

#[derive(Debug, Serialize)]
struct Output {
    pending: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let state = state_handle(&input.state);
        let pending = compaction_pending(state.as_ref(), &input.payload);
        serde_json::to_value(Output { pending }).map_err(|error| {
            Box::new(HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

// LiteLLM's helper builds CompactionState(config=ContextCompactionConfig() if armed else None)
fn state_handle(state: &State) -> Option<SharedCompactionState<()>> {
    let config = match state {
        State::Absent => return None,
        State::Unarmed => None,
        State::Armed => Some(ContextCompactionConfig::default()),
    };
    let shared = CompactionState {
        config,
        candidates: Vec::new(),
        summary: None,
        parent_model: None,
        surface: None,
    };
    Some(Arc::new(Mutex::new(shared)))
}
