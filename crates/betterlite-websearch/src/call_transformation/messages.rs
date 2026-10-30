use super::TransformationError;
use super::arguments::wire_text;
use super::wire::{
    Assistant, AssistantBody, Call, CallKind, Content, Results, ToolMessage, ToolResult,
    UserMessage, WireCall, WireFunction,
};

pub(super) fn assistant(calls: &[Call], format: &str) -> Result<Assistant, TransformationError> {
    let body = if format == "openai" {
        let tool_calls = calls
            .iter()
            .map(|call| {
                Ok(WireCall {
                    id: call.id.clone(),
                    kind: CallKind::Function,
                    function: WireFunction {
                        name: call.name.clone(),
                        arguments: wire_text(&call.input)?,
                    },
                })
            })
            .collect::<Result<Vec<_>, TransformationError>>()?;
        AssistantBody::OpenAi { tool_calls }
    } else {
        let content = calls
            .iter()
            .map(|call| Content::ToolUse {
                kind: CallKind::ToolUse,
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
            })
            .collect();
        AssistantBody::Anthropic { content }
    };
    Ok(Assistant::Assistant { body })
}

pub(super) fn results(calls: &[Call], search_results: &[String], format: &str) -> Results {
    if format == "openai" {
        let messages = calls
            .iter()
            .zip(search_results)
            .map(|(call, result)| ToolMessage::Tool {
                tool_call_id: call.id.clone(),
                content: result.clone(),
            })
            .collect();
        Results::OpenAi(messages)
    } else {
        let content = calls
            .iter()
            .zip(search_results)
            .map(|(call, result)| ToolResult::ToolResult {
                tool_use_id: call.id.clone(),
                content: result.clone(),
            })
            .collect();
        Results::Anthropic(UserMessage::User { content })
    }
}
