use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_provider::replicate::request::ReplicateDefaults;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

use self::script::{FIXED_NOW, ScriptedResponse};

mod ops;
mod poll_ops;
mod script;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    MapParams { cases: Vec<MapCase> },
    Urls { models: Vec<String> },
    Headers { requests: Vec<HeaderRequest> },
    Request { requests: Vec<RequestCase> },
    Reply { replies: Vec<ReplyCase> },
    Poll { requests: Vec<PollCase> },
    Stream { requests: Vec<StreamCase> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MapCase {
    pub(super) model: String,
    #[serde(default)]
    pub(super) non_default: Object,
    #[serde(default)]
    pub(super) optional_params: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HeaderRequest {
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestCase {
    pub(super) model: String,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) optional_params: Object,
    #[serde(default)]
    pub(super) defaults: ReplicateDefaults,
    #[serde(default)]
    pub(super) custom_prompt_dict: BTreeMap<String, CustomPromptTemplate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyCase {
    pub(super) model: String,
    pub(super) body: String,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PollCase {
    pub(super) model: String,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) optional_params: Object,
    #[serde(default)]
    pub(super) defaults: ReplicateDefaults,
    #[serde(default)]
    pub(super) custom_prompt_dict: BTreeMap<String, CustomPromptTemplate>,
    #[serde(default = "default_retries")]
    pub(super) retries: i32,
    #[serde(default = "default_retry_delay")]
    pub(super) retry_delay: f64,
    #[serde(default = "default_now")]
    pub(super) now: i64,
    pub(super) create: ScriptedResponse,
    #[serde(default)]
    pub(super) polls: Vec<ScriptedResponse>,
}

impl PollCase {
    pub(super) fn scripted(&self) -> Vec<ScriptedResponse> {
        let mut responses = Vec::with_capacity(self.polls.len().saturating_add(1));
        responses.push(self.create.clone());
        responses.extend(self.polls.iter().cloned());
        responses
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StreamCase {
    pub(super) policy: Policy,
    pub(super) url: String,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default = "default_polling_delay")]
    pub(super) polling_delay: f64,
    #[serde(default)]
    pub(super) polls: Vec<ScriptedResponse>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Policy {
    Strict,
    Lenient,
}

const fn default_retries() -> i32 {
    5
}

const fn default_retry_delay() -> f64 {
    1.0_f64
}

const fn default_now() -> i64 {
    FIXED_NOW
}

const fn default_polling_delay() -> f64 {
    0.5_f64
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MapParams { cases } => Ok(ops::map_params(cases)),
            Input::Urls { models } => Ok(ops::urls(models)),
            Input::Headers { requests } => ops::headers(requests),
            Input::Request { requests } => ops::requests(requests),
            Input::Reply { replies } => ops::replies(replies),
            Input::Poll { requests } => poll_ops::poll(requests),
            Input::Stream { requests } => poll_ops::streams(requests),
        }
    }
}
