mod ops;
mod support;

use serde_json::Value;

use crate::case::UpstreamAnswer;
use crate::helpers::HelperResult;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreCallInput {
    settings: Value,
    request_data: Value,
    #[serde(default)]
    call_type: Option<String>,
    #[serde(default = "default_true")]
    should_run: bool,
    #[serde(default)]
    key_user_id: Option<String>,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostCallInput {
    settings: Value,
    request_data: Value,
    response: Value,
    #[serde(default = "default_true")]
    should_run: bool,
    #[serde(default)]
    key_user_id: Option<String>,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatInput {
    settings: Value,
    request_data: Value,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    key_user_id: Option<String>,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpRequestInput {
    settings: Value,
    request_data: Value,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpResponseInput {
    settings: Value,
    request_data: Value,
    response: Value,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostInspectionInput {
    settings: Value,
    #[serde(default)]
    payload: Value,
    #[serde(default = "default_surface")]
    surface: String,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Construct { settings: Value },
    BuildHeaders { settings: Value },
    PreCallHook(PreCallInput),
    ModerationHook(PreCallInput),
    PostCallHook(PostCallInput),
    InspectChat(ChatInput),
    InspectMcpRequest(McpRequestInput),
    InspectMcpResponse(McpResponseInput),
    PostInspection(PostInspectionInput),
}

const fn default_true() -> bool {
    true
}

fn default_surface() -> String {
    "chat".to_owned()
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Construct { settings } => ops::construct(settings),
            Input::BuildHeaders { settings } => ops::headers(settings),
            Input::PreCallHook(args) => ops::pre_call_hook(args, false),
            Input::ModerationHook(args) => ops::pre_call_hook(args, true),
            Input::PostCallHook(args) => ops::post_call_hook(args),
            Input::InspectChat(args) => ops::chat(args),
            Input::InspectMcpRequest(args) => ops::mcp_request(args),
            Input::InspectMcpResponse(args) => ops::mcp_response(args),
            Input::PostInspection(args) => ops::post_inspection_op(args),
        }
    }
}

mod tests;
