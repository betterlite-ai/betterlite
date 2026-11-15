use crate::response_tool_calls::normalized::get_tool_calls_from_response;
use crate::response_tool_calls::projections::{
    ChatChoice, ChatMessage, ChatToolCall, ToolCallResponse,
};

mod tests;

// Every choice of an n>1 response cost money and its tool calls ran, so all of them count
#[must_use]
pub fn response_tool_call_names(response: &ToolCallResponse) -> Vec<String> {
    let mut names = Vec::new();
    for tool_call in get_tool_calls_from_response(response, true) {
        let Some(name) = tool_call.name else {
            continue;
        };
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            names.push(trimmed.to_owned());
        }
    }
    names
}

// A realtime session carries its invoked tools as a kwargs list, not on a response object
#[must_use]
pub fn realtime_tool_call_names(tool_calls: &[ChatToolCall]) -> Vec<String> {
    let response = ToolCallResponse {
        choices: vec![ChatChoice {
            message: Some(ChatMessage {
                tool_calls: tool_calls.to_vec(),
            }),
        }],
        output: Vec::new(),
        content: Vec::new(),
    };
    response_tool_call_names(&response)
}
