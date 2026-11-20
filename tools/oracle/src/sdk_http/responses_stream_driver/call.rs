use crate::sdk_http::result::Failure;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
pub type Outcome = Vec<RunOutcome>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Live,
    Scripted,
    Fake,
    Cached,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Next,
    Drain,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Read,
    Protocol,
    Timeout,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptedFailure {
    pub kind: FailureKind,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub model: String,
    pub input: Json,
    pub api_key: String,
    pub timeout_seconds: u64,
    pub runs: Vec<RunSpec>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub mode: Mode,
    pub asynchronous: bool,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub payloads: Vec<String>,
    #[serde(default)]
    pub failure: Option<ScriptedFailure>,
    #[serde(default)]
    pub response: Option<Json>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOutcome {
    pub events: Vec<Json>,
    pub end: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub steps: Vec<StepOutcome>,
    pub failure: Option<Failure>,
    pub effects: Vec<String>,
    pub logged: Vec<String>,
}
