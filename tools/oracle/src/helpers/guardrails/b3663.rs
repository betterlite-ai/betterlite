use betterlite_server::guardrail_message_scope::assistant_text::assistant_text_from_response;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

mod failures;
mod scope;
mod selection;
mod text;
mod usage;

#[path = "b3663/guardrail_message_scope/tests.rs"]
mod guardrail_message_scope;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    BlockedUsage(usage::Request),
    Slots(scope::Slots),
    Scope(scope::Scope),
    ToolNames(scope::ToolNames),
    MergeTools(scope::MergeTools),
    MergeMessages(scope::MergeMessages),
    ContentText(text::ContentText),
    AssistantText(text::AssistantText),
    IsHttpException(failures::IsHttpException),
    EnrichException(failures::EnrichException),
    PreCall(failures::PreCall),
    Unappliable(failures::Unappliable),
    CanModify(selection::CanModify),
    Proceed(selection::Proceed),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BlockedUsage(request) => Ok(usage::blocked_usage(request)),
            Input::Slots(request) => Ok(scope::slots(request)),
            Input::Scope(request) => Ok(scope::scope(request)),
            Input::ToolNames(request) => Ok(scope::tool_names(request)),
            Input::MergeTools(request) => Ok(scope::merge_tools(request)),
            Input::MergeMessages(request) => Ok(scope::merge_messages(request)),
            Input::ContentText(request) => Ok(text::content_text(request)),
            Input::AssistantText(request) => Ok(json!({
                "text": assistant_text_from_response(&request.response),
            })),
            Input::IsHttpException(request) => Ok(failures::is_http_exception(request)),
            Input::EnrichException(request) => Ok(failures::enrich_exception(request)),
            Input::PreCall(request) => failures::pre_call(request),
            Input::Unappliable(request) => Ok(failures::unappliable(request)),
            Input::CanModify(request) => Ok(selection::can_modify(request)),
            Input::Proceed(request) => Ok(selection::proceed(request)),
        }
    }
}
