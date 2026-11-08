use betterlite_router::routing_timeouts::completion::TimeoutValue;
use betterlite_router::routing_timeouts::seconds::Seconds;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    #[serde(default)]
    pub configured: Option<ConfiguredInput>,
    #[serde(default)]
    pub completion: Option<CompletionInput>,
    #[serde(default)]
    pub router: Option<RouterInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfiguredInput {
    pub seconds: String,
    pub explicitly_set: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompletionInput {
    #[serde(default)]
    pub model_timeout: Option<TimeoutInput>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub request_timeout: Option<String>,
    pub provider: String,
    pub global_timeout: GlobalInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GlobalInput {
    pub seconds: String,
    pub explicitly_set: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum TimeoutInput {
    Seconds {
        seconds: String,
    },
    Phases {
        #[serde(default)]
        connect: Option<String>,
        #[serde(default)]
        read: Option<String>,
        #[serde(default)]
        write: Option<String>,
        #[serde(default)]
        pool: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RouterInput {
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub stream_timeout: Option<String>,
    #[serde(default)]
    pub default_stream_timeout: Option<String>,
    pub global_timeout: GlobalInput,
    #[serde(default)]
    pub kwargs: KwargsInput,
    #[serde(default)]
    pub deployment: DeploymentInput,
    #[serde(default)]
    pub elapsed_millis: Option<u64>,
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KwargsInput {
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub request_timeout: Option<String>,
    #[serde(default)]
    pub stream_timeout: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeploymentInput {
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub request_timeout: Option<String>,
    #[serde(default)]
    pub stream_timeout: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Fields {
    pub configured: Option<String>,
    pub resolved: Option<TimeoutOutput>,
    pub transport: Option<TimeoutOutput>,
    pub stream: Option<TimeoutOutput>,
    pub non_stream: Option<TimeoutOutput>,
    pub attempt: Option<TimeoutOutput>,
    pub deadline_millis: Option<u64>,
    pub attempt_started: Option<bool>,
    pub expired_status: Option<u16>,
    pub expired_code: Option<String>,
    pub cancelled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum TimeoutOutput {
    Seconds {
        seconds: String,
    },
    Phases {
        connect: Option<String>,
        read: Option<String>,
        write: Option<String>,
        pool: Option<String>,
    },
}

impl From<Seconds> for TimeoutOutput {
    fn from(seconds: Seconds) -> Self {
        Self::Seconds {
            seconds: format!("{:.6}", seconds.0),
        }
    }
}

impl From<TimeoutValue> for TimeoutOutput {
    fn from(value: TimeoutValue) -> Self {
        match value {
            TimeoutValue::Seconds(seconds) => Self::from(seconds),
            TimeoutValue::Phases(phases) => Self::Phases {
                connect: phases.connect.map(|seconds| format!("{:.6}", seconds.0)),
                read: phases.read.map(|seconds| format!("{:.6}", seconds.0)),
                write: phases.write.map(|seconds| format!("{:.6}", seconds.0)),
                pool: phases.pool.map(|seconds| format!("{:.6}", seconds.0)),
            },
        }
    }
}
