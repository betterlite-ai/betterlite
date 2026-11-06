mod config;
mod hooks;

use betterlite_guardrails::ibm_hooks::config::EventHook;
use serde_json::Value;

use crate::case::UpstreamAnswer;
use crate::helpers::HelperResult;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HookInput {
    pub(crate) settings: config::Settings,
    pub(crate) data: Value,
    pub(crate) answer: UpstreamAnswer,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostCallInput {
    pub(crate) settings: config::Settings,
    pub(crate) data: Value,
    pub(crate) response: Value,
    pub(crate) answer: UpstreamAnswer,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamingInput {
    pub(crate) items: Vec<Value>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShouldRunInput {
    pub(crate) settings: config::Settings,
    pub(crate) event_type: EventHook,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    PreCall(HookInput),
    DuringCall(HookInput),
    PostCall(PostCallInput),
    Streaming(StreamingInput),
    ShouldRun(ShouldRunInput),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::PreCall(args) => hooks::input_hook(args, hooks::InputEvent::PreCall),
            Input::DuringCall(args) => hooks::input_hook(args, hooks::InputEvent::DuringCall),
            Input::PostCall(args) => hooks::post_call(args),
            Input::Streaming(args) => hooks::streaming(args),
            Input::ShouldRun(args) => hooks::should_run(args),
        }
    }
}
