use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesStreamLoggingCall {
    pub(crate) body: IndexMap<String, Json>,
    pub(crate) steps: Vec<LoggingStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum LoggingStep {
    Collect {
        first_chunk: bool,
    },
    Bill {
        teardown: bool,
        #[serde(default)]
        route_failure: Option<String>,
    },
    ArmDeferred,
    ParkArgs {
        count: usize,
    },
    Dispatch,
    AbortUpstream {
        close: Close,
    },
    DetachedFailure {
        hook: Hook,
    },
    Convert {
        chunk: ChunkSpec,
    },
    Delegate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Close {
    Unsupported,
    Closes,
    Fails,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hook {
    Missing,
    Records,
    Raises,
}

// An object is a dict chunk and a string is a bytes chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChunkSpec {
    Mapping(IndexMap<String, Json>),
    Bytes(String),
}
