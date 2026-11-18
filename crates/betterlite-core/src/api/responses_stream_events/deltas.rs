use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

use crate::api::responses_stream_events::JsonShape;
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;

// only these delta events feed the generated text or arguments, in the order the stream delivered them
pub fn accumulate(event: &ResponsesAPIStreamingResponse, state: &mut ResponsesStreamBaseState) {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => {
            if let KnownResponseEvent::OutputTextDelta(record) = known.as_ref() {
                state.generated_content.push_str(&record.delta);
            }
            if let KnownResponseEvent::FunctionCallArgumentsDelta(record) = known.as_ref() {
                state.generated_tool_arguments.push_str(&record.delta);
            }
            if let KnownResponseEvent::McpCallArgumentsDelta(record) = known.as_ref() {
                state.generated_tool_arguments.push_str(&record.delta);
            }
        }
        ResponsesAPIStreamingResponse::Generic(record) => {
            let Some(text) = record.extra.get("delta").and_then(JsonShape::optional_str) else {
                return;
            };
            match record.r#type.as_str() {
                "response.output_text.delta" => state.generated_content.push_str(text),
                "response.function_call_arguments.delta"
                | "response.custom_tool_call_input.delta"
                | "response.mcp_call_arguments.delta" => {
                    state.generated_tool_arguments.push_str(text);
                }
                _ => {}
            }
        }
    }
}
