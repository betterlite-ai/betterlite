use std::collections::BTreeMap;

use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_openai::thread_runs::records::RunRequest;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fixture;
mod reports;
mod steps;
mod tests;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KindName {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SuppliedStep {
    pub(crate) kind: KindName,
}

// The run keywords the caller hands the facade, keyed as LiteLLM's signature names them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunSpec {
    thread_id: String,
    assistant_id: String,
    #[serde(default)]
    additional_instructions: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Option<Value>,
}

impl RunSpec {
    pub(crate) fn request(&self) -> RunRequest {
        RunRequest {
            thread_id: self.thread_id.clone(),
            assistant_id: self.assistant_id.clone(),
            additional_instructions: self.additional_instructions.clone(),
            instructions: self.instructions.clone(),
            metadata: self.metadata.clone(),
            model: self.model.clone(),
            tools: self.tools.clone(),
        }
    }
}

// One facade call: the method arguments LiteLLM reads, the litellm_params it forwards, and the SDK replies to serve.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    #[serde(default)]
    params: LiteLlmParams,
    #[serde(default)]
    api_key: Option<SecretString>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_version: Option<String>,
    #[serde(default)]
    azure_ad_token: Option<SecretString>,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    max_retries: Option<i64>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    arun_thread: Option<Value>,
    #[serde(default)]
    stream: Option<Value>,
    #[serde(default)]
    handler: Option<bool>,
    run: RunSpec,
    #[serde(default)]
    supplied: Option<SuppliedStep>,
    #[serde(default)]
    replies: Vec<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    RunPoll(Vec<Step>),
    RunStream(Vec<Step>),
    RunInstructionsTools(Vec<Step>),
    RunFailure(Vec<Step>),
    RunDispatch(Vec<Step>),
    InstructionsToolsOverrides(Vec<Step>),
    StreamEventHandler(Vec<Step>),
    AssistantsBetaDefault(Vec<Step>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}
