use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamLoggingRun {
    pub steps: Vec<StepOutcome>,
}

// What one step changed, read after the logging worker drained
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    pub collected: usize,
    pub routed: Vec<Routed>,
    pub processed: Vec<Processed>,
    pub ran: usize,
    pub warnings: Vec<String>,
    pub callback_args: Vec<usize>,
    pub hook_calls: Vec<String>,
    pub completion_start_recorded: bool,
    pub callback_armed: bool,
    pub args_present: bool,
    pub frame: Option<String>,
    pub provider_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routed {
    pub raw_frames: Vec<String>,
    pub request_body: IndexMap<String, Json>,
    pub endpoint_anthropic: bool,
    pub url_route: String,
    pub iterator_logging_obj: bool,
    pub shared_success_handler: bool,
    pub start_is_construction: bool,
    pub end_not_before_start: bool,
    pub first_within_run: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Processed {
    pub response_status: u16,
    pub request_body: IndexMap<String, Json>,
    pub endpoint_anthropic: bool,
    pub url_route: String,
    pub supplied_logging_obj: bool,
    pub shared_success_handler: bool,
    pub start_is_construction: bool,
}
