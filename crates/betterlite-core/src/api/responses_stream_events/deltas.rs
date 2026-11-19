use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

use crate::api::responses_stream_events::JsonShape;
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;

// only these delta events feed the generated text or arguments, in the order the stream delivered them
pub fn accumulate(event: &ResponsesAPIStreamingResponse, state: &mut ResponsesStreamBaseState) {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => match known.as_ref() {
            KnownResponseEvent::OutputTextDelta(record) => {
                state.generated_content.push_str(&record.delta);
            }
            KnownResponseEvent::FunctionCallArgumentsDelta(record)
            | KnownResponseEvent::McpCallArgumentsDelta(record) => {
                state.generated_tool_arguments.push_str(&record.delta);
            }
            _ => {}
        },
        ResponsesAPIStreamingResponse::Generic(record) => {
            let delta = match record.r#type.as_str() {
                "response.output_text.delta" => &mut state.generated_content,
                "response.function_call_arguments.delta"
                | "response.custom_tool_call_input.delta"
                | "response.mcp_call_arguments.delta" => &mut state.generated_tool_arguments,
                _ => return,
            };
            if let Some(text) = record.extra.get("delta").and_then(JsonShape::optional_str) {
                delta.push_str(text);
            }
        }
    }
}
