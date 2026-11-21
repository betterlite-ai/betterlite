use betterlite_router::retry_policy::policy::RetryPolicy;
use serde::{Deserialize, Serialize};

use crate::router_probes::retry_policy::ErrorInput;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub deployments: Vec<Deployment>,
    pub num_retries: i64,
    #[serde(default)]
    pub request_num_retries: Option<i64>,
    #[serde(default)]
    pub retry_policy: Option<RetryPolicy>,
    pub timeout: String,
    #[serde(default)]
    pub stream: bool,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub id: String,
    #[serde(default)]
    pub num_retries: Option<u32>,
}

// one attempt's answer: an error, a disconnect while it runs, or a success when neither is set
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub error: Option<ErrorInput>,
    #[serde(default)]
    pub disconnect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Answered,
    Failed,
    Cancelled,
    TimedOut,
}

// the deployment one attempt reached and the retry metadata its request carried
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub deployment: String,
    pub attempted_retries: i64,
    pub max_retries: i64,
    pub breadcrumbs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Headers {
    pub attempted_retries: i64,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status: u16,
    pub code: String,
    pub max_retries: Option<u32>,
    pub num_retries: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    pub outcome: Outcome,
    pub calls: Vec<Call>,
    pub backoffs: Vec<String>,
    pub headers: Option<Headers>,
    pub error: Option<Failure>,
}
