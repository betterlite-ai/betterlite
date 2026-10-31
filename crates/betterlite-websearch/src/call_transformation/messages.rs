use super::TransformationError;
use super::arguments::wire_text;
use super::wire::{
    Assistant, AssistantBody, Call, CallKind, Content, ResponseFormat, Results, ToolMessage,
    ToolResult, UserMessage, WireCall, WireFunction,
};

impl TryFrom<&Call> for WireFunction {
    type Error = TransformationError;

    fn try_from(call: &Call) -> Result<Self, Self::Error> {
        Ok(Self {
            name: call.name.clone(),
            arguments: wire_text(&call.input)?,
        })
    }
}

impl TryFrom<&Call> for WireCall {
    type Error = TransformationError;

    fn try_from(call: &Call) -> Result<Self, Self::Error> {
        Ok(Self {
            id: call.id.clone(),
            kind: CallKind::Function,
            function: WireFunction::try_from(call)?,
        })
    }
}

impl From<&Call> for Content {
    fn from(call: &Call) -> Self {
        Self::ToolUse {
            kind: CallKind::ToolUse,
            id: call.id.clone(),
            name: call.name.clone(),
            input: call.input.clone(),
        }
    }
}

impl From<(&Call, &String)> for ToolMessage {
    fn from((call, result): (&Call, &String)) -> Self {
        Self::Tool {
            tool_call_id: call.id.clone(),
            content: result.clone(),
        }
    }
}

impl From<(&Call, &String)> for ToolResult {
    fn from((call, result): (&Call, &String)) -> Self {
        Self::ToolResult {
            tool_use_id: call.id.clone(),
            content: result.clone(),
        }
    }
}

pub(super) fn assistant_and_results(
    calls: &[Call],
    search_results: &[String],
    format: ResponseFormat,
) -> Result<(Assistant, Results), TransformationError> {
    match format {
        ResponseFormat::OpenAi => {
            let tool_calls = calls
                .iter()
                .map(WireCall::try_from)
                .collect::<Result<Vec<_>, TransformationError>>()?;
            let messages = calls
                .iter()
                .zip(search_results)
                .map(ToolMessage::from)
                .collect();
            Ok((
                Assistant::Assistant {
                    body: AssistantBody::OpenAi { tool_calls },
                },
                Results::OpenAi(messages),
            ))
        }
        ResponseFormat::Anthropic | ResponseFormat::Responses => {
            let content = calls.iter().map(Content::from).collect();
            let result_content = calls
                .iter()
                .zip(search_results)
                .map(ToolResult::from)
                .collect();
            Ok((
                Assistant::Assistant {
                    body: AssistantBody::Anthropic { content },
                },
                Results::Anthropic(UserMessage::User {
                    content: result_content,
                }),
            ))
        }
    }
}
