use betterlite_router::routing_value_store::SchedulerTuple;
use serde::{Deserialize, Serialize};
pub(super) const MODEL: &str = "gpt-4o";
fn default_model() -> String {
    MODEL.to_owned()
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    // the string form keeps the payload Eq, and the scenario parses it
    #[serde(default)]
    pub polling_interval: Option<String>,
    #[serde(default)]
    pub default_priority: Option<u8>,
    pub timeout: String,
    #[serde(default)]
    pub fill: Option<Fill>,
    pub steps: Vec<Step>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Fill {
    pub count: usize,
    pub prefix: String,
    #[serde(default)]
    pub priority: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Step {
    Add {
        priority: u8,
        request_id: String,
        #[serde(default = "default_model")]
        model: String,
    },
    Poll {
        request_id: String,
        #[serde(default = "default_model")]
        model: String,
        healthy: usize,
    },
    Peek {
        request_id: String,
        #[serde(default = "default_model")]
        model: String,
    },
    Queue {
        #[serde(default = "default_model")]
        model: String,
    },
    Interval {
        #[serde(default = "default_model")]
        model: String,
    },
    Schedule {
        priority: u8,
        request_id: String,
        #[serde(default = "default_model")]
        model: String,
        healthy: usize,
    },
}
impl Step {
    pub(super) fn model(&self) -> &str {
        match self {
            Self::Add { model, .. }
            | Self::Poll { model, .. }
            | Self::Peek { model, .. }
            | Self::Queue { model }
            | Self::Interval { model }
            | Self::Schedule { model, .. } => model,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Outcome {
    Added,
    Polled { allowed: bool },
    Peeked { head: bool },
    Queue,
    Interval { interval: String },
    Scheduled { prioritized: bool, priority: u8 },
    Error,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Failure {
    pub status: Option<u16>,
    pub code: Option<String>,
    pub priority: Option<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StepOutput {
    pub outcome: Outcome,
    pub queue: Vec<SchedulerTuple>,
    // the queue's length after the step, which a case asserts where the full list is too large to spell out
    pub depth: usize,
    pub error: Option<Failure>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OutputFields {
    pub steps: Vec<StepOutput>,
    // the provider calls the scheduled requests made, in total
    pub calls: usize,
}
