use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// A result boolean that serializes as true or false while keeping the struct under the bool limit
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Flag(pub bool);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamLoggingRun {
    pub steps: Vec<StepOutcome>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    pub routed: Vec<Routed>,
    pub processed: Vec<Processed>,
    pub ran: usize,
    pub warnings: Vec<String>,
    pub callback_args: Vec<usize>,
    pub hook_calls: Vec<String>,
    pub completion_start_recorded: Flag,
    pub callback_armed: Flag,
    pub args_present: Flag,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routed {
    pub raw_frames: Vec<String>,
    pub request_body: IndexMap<String, Json>,
    pub endpoint_anthropic: Flag,
    pub url_route: String,
    pub iterator_logging_obj: Flag,
    pub shared_success_handler: Flag,
    pub start_is_construction: Flag,
    pub first_within_run: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Processed {
    pub response_status: u16,
    pub request_body: IndexMap<String, Json>,
    pub endpoint_anthropic: Flag,
    pub url_route: String,
    pub supplied_logging_obj: Flag,
    pub shared_success_handler: Flag,
    pub start_is_construction: Flag,
}
