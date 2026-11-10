use betterlite_openai::chat::ToolCall;
use betterlite_openai::stream::chunk::{Function, ToolCall as StreamToolCall};
pub(crate) fn stream_tool_calls(calls: Vec<ToolCall>) -> Vec<StreamToolCall> {
    calls
        .into_iter()
        .enumerate()
        .map(|(position, call)| StreamToolCall {
            index: u32::try_from(position).unwrap_or(u32::MAX),
            id: Some(call.id),
            kind: Some(call.kind),
            function: Some(Function {
                name: Some(call.function.name),
                arguments: call.function.arguments,
            }),
        })
        .collect()
}
