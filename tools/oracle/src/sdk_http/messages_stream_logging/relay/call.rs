use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::super::outcome::Flag;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesNativeStreamCall {
    pub(crate) body: IndexMap<String, Json>,
    pub(crate) queue_maxsize: usize,
    pub(crate) max_detached_stream_drains: usize,
    pub(crate) steps: Vec<RelayStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RelayStep {
    Start { stream: String },
    Release { stream: String, count: usize },
    Read { stream: String },
    Detach { stream: String },
    Finish { stream: String },
    Fail { stream: String },
    Arm { stream: String },
    Wait { stream: String, until: WaitUntil },
    Settle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitUntil {
    Idle,
    Parked,
    Hook,
}

impl RelayStep {
    pub(super) fn stream(&self) -> &str {
        match self {
            Self::Start { stream }
            | Self::Release { stream, .. }
            | Self::Read { stream }
            | Self::Detach { stream }
            | Self::Finish { stream }
            | Self::Fail { stream }
            | Self::Arm { stream }
            | Self::Wait { stream, .. } => stream,
            Self::Settle => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStreamRun {
    pub steps: Vec<StepOutcome>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    pub frame: Option<FrameRead>,
    pub observation: Option<Observation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameRead {
    pub chunk: Option<String>,
    pub end: Flag,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub warnings: Vec<String>,
    pub streams: IndexMap<String, StreamObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamObservation {
    pub billed: Vec<Vec<String>>,
    pub failures: Vec<FailureCall>,
    pub hook_calls: Vec<String>,
    pub callback_args: Vec<usize>,
    pub ran: usize,
    pub armed: Flag,
    pub parked: Flag,
    pub closed: Flag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureCall {
    pub kind: String,
    pub message: String,
    pub endpoint_anthropic: Flag,
    pub request_body: IndexMap<String, Json>,
    pub raw_frames: Vec<String>,
}
