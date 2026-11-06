use std::collections::BTreeMap;

use betterlite_openai::thread_runs::records::RunRequest;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OptionFields {
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) timeout: Option<f64>,
    pub(super) max_retries: Option<u32>,
    pub(super) organization: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(super) status: u16,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
}
// The switch fields stay as raw JSON so a case can pass the non-boolean values LiteLLM's `is True` tests refuse.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Round {
    #[serde(default)]
    pub(super) arun_thread: Option<Value>,
    #[serde(default)]
    pub(super) stream: Option<Value>,
    #[serde(default)]
    pub(super) handler: bool,
    pub(super) request: RunRequest,
    #[serde(default)]
    pub(super) replies: Vec<Reply>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fields {
    #[serde(default)]
    pub(super) options: OptionFields,
    pub(super) rounds: Vec<Round>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadRunsCall {
    RunPollCompletion(Fields),
    RequiresAction(Fields),
    FailedRun(Fields),
    StreamIncrementalEvents(Fields),
    InstructionsToolsOverrides(Fields),
    RunDispatch(Fields),
    StreamEventHandler(Fields),
    AssistantsBetaDefault(Fields),
}
