use betterlite_context::provider_message_primitives::FunctionKind;

use super::StreamEventConverter;
use super::chunk::{Branch, StreamEventToolCall};
use super::decode_stream_tool_arguments;
use crate::stream_state::events::ToolFunctionChunk;
use crate::stream_state::state::{StreamState, check_empty_tool_call_args};

// ContentBlockStop(**chunk) never validates, so the stop event itself is not read.
pub(crate) fn content_block_stop<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    branch: &mut Branch,
) {
    let block_type = converter.state.current_content_block_type.clone();
    let tool_block = matches!(block_type.as_deref(), Some("tool_use" | "server_tool_use"));
    if tool_block && check_empty_tool_call_args(&converter.state) {
        branch.tool_call = Some(StreamEventToolCall {
            id: None,
            kind: FunctionKind::Function,
            function: ToolFunctionChunk {
                name: None,
                arguments: "{}".to_owned(),
            },
            index: converter.state.tool_index,
            caller: None,
        });
    }
    if block_type.as_deref() == Some("server_tool_use")
        && let Some(id) = converter.state.current_server_tool_id.take()
    {
        let arguments = assembled_arguments(&converter.state);
        if let Ok(parsed) = decode_stream_tool_arguments(&arguments) {
            converter.state.server_tool_inputs.insert(id, parsed);
        }
    }
    converter.state.is_response_format_tool = false;
    converter.state.current_content_block_type = None;
}

pub(super) fn assembled_arguments(state: &StreamState) -> String {
    let mut arguments = String::new();
    for block in &state.content_blocks {
        if block.delta.kind == "input_json_delta"
            && let Some(partial) = &block.delta.partial_json
        {
            arguments.push_str(partial);
        }
    }
    arguments
}
