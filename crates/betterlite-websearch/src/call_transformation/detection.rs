use super::TransformationError;
use super::arguments::{normalize, wire_text};
use super::wire::{
    Arguments, Block, Call, CallFunction, CallKind, Input, Response, ResponseFormat,
};

pub(super) fn detect(
    response: &Response,
    format: ResponseFormat,
) -> Result<Vec<Call>, TransformationError> {
    let (kind, blocks) = response_blocks(response, format);
    let mut calls = Vec::new();
    for block in blocks {
        let (tag, candidate_name) = match kind {
            CallKind::ToolUse => ("tool_use", block.name.as_deref()),
            CallKind::Function => (
                "function",
                block
                    .function
                    .as_ref()
                    .and_then(|function| function.name.as_deref()),
            ),
            CallKind::FunctionCall => ("function_call", block.name.as_deref()),
        };
        let Some(name) = candidate_name else {
            continue;
        };
        let matches_name = name == "litellm_web_search"
            || (name == "web_search" && !matches!(kind, CallKind::FunctionCall));
        if block.kind.as_deref() == Some(tag) && matches_name {
            calls.push(extract(block, kind, name)?);
        }
    }
    Ok(calls)
}

fn response_blocks(response: &Response, format: ResponseFormat) -> (CallKind, &[Block]) {
    match format {
        ResponseFormat::OpenAi => {
            let blocks = response
                .choices
                .first()
                .and_then(|choice| choice.message.as_ref())
                .map_or(&[][..], |message| message.tool_calls.0.as_slice());
            (CallKind::Function, blocks)
        }
        ResponseFormat::Responses => (CallKind::FunctionCall, response.output.0.as_slice()),
        ResponseFormat::Anthropic => (CallKind::ToolUse, response.content.0.as_slice()),
    }
}

fn extract(block: &Block, kind: CallKind, name: &str) -> Result<Call, TransformationError> {
    let input = match kind {
        CallKind::ToolUse => block.input.clone(),
        CallKind::Function => block
            .function
            .as_ref()
            .map_or_else(Input::default, |function| {
                normalize(&function.arguments, false)
            }),
        CallKind::FunctionCall => normalize(&block.arguments, true),
    };
    let (id, arguments, call_id) = match kind {
        CallKind::FunctionCall => {
            let text = match &block.arguments {
                Arguments::Present(raw) => serde_json::from_str::<String>(raw.get())
                    .map_or_else(|_| wire_text(&input), Ok)?,
                Arguments::Missing => String::new(),
            };
            (
                block.call_id.clone(),
                Some(text),
                Some(block.call_id.clone()),
            )
        }
        CallKind::ToolUse | CallKind::Function => (block.id.clone(), None, None),
    };
    let function = matches!(kind, CallKind::Function).then(|| CallFunction {
        name: name.to_owned(),
        arguments: input.clone(),
    });
    Ok(Call {
        id,
        kind,
        name: name.to_owned(),
        input,
        function,
        arguments,
        call_id,
    })
}
