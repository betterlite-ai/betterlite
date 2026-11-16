use serde_json::value::RawValue;

use crate::prisma_airs_extraction as extraction;
use crate::prisma_airs_masking as masking;

// LiteLLM's isinstance(response, ModelResponse) check: anything else passes through untouched.
pub enum CallResponse<'response> {
    Model(&'response mut masking::Response),
    Other,
}

// B3787's extraction and masking views read the same wire response; this is the one conversion between them.
#[must_use]
pub fn extraction_view(response: &masking::Response) -> extraction::Response {
    match response.choices.as_ref() {
        Some(choices) => extraction::Response {
            choices: Some(choices_view(choices)),
        },
        None => extraction::Response { choices: None },
    }
}

fn choices_view(choices: &[masking::ResponseChoice]) -> Vec<extraction::ResponseChoice> {
    let mut views = Vec::with_capacity(choices.len());
    for choice in choices {
        views.push(choice_view(choice));
    }
    views
}

fn choice_view(choice: &masking::ResponseChoice) -> extraction::ResponseChoice {
    match choice.message.as_ref() {
        Some(message) => extraction::ResponseChoice {
            message: Some(message_view(message)),
        },
        None => extraction::ResponseChoice { message: None },
    }
}

fn message_view(message: &masking::MaskedResponseMessage) -> extraction::ResponseMessage {
    extraction::ResponseMessage {
        content: message.content.as_ref().map(content_view),
        tool_calls: message.tool_calls.as_ref().map(tool_call_views),
        function_call: message.function_call.as_ref().map(function_view),
    }
}

fn content_view(content: &masking::MaskContent) -> extraction::MessageContent {
    match content {
        masking::MaskContent::Text(text) => extraction::MessageContent::Text(text.clone()),
        masking::MaskContent::Parts(parts) => {
            extraction::MessageContent::Parts(parts.iter().filter_map(part_view).collect())
        }
    }
}

fn tool_call_views(calls: &[masking::MaskedToolCall]) -> Vec<extraction::ToolCall> {
    let mut views = Vec::with_capacity(calls.len());
    for call in calls {
        views.push(tool_call_view(call));
    }
    views
}

fn tool_call_view(call: &masking::MaskedToolCall) -> extraction::ToolCall {
    match call.function.as_ref() {
        Some(function) => extraction::ToolCall {
            function: Some(function_view(function)),
        },
        None => extraction::ToolCall { function: None },
    }
}

fn function_view(call: &masking::MaskedFunctionCall) -> extraction::FunctionCall {
    match call.arguments.as_ref() {
        Some(arguments) => extraction::FunctionCall {
            arguments: Some(arguments.clone()),
        },
        None => extraction::FunctionCall { arguments: None },
    }
}

// A content part LiteLLM would not read as text, such as an image, stays out of the scan.
fn part_view(raw: &RawValue) -> Option<extraction::ContentPart> {
    match serde_json::from_str(raw.get()) {
        Ok(part) => Some(part),
        Err(_) => None,
    }
}
