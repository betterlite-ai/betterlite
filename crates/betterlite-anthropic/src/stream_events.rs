use betterlite_models::decisions::Json;

use crate::stream_state::json_fragment_buffer::AccumulatedJson as _;
use crate::stream_state::state::StreamState;
use crate::usage::ReasoningTokenCounter;

mod accumulated;
pub mod chunk;
mod dispatch;
pub mod error;
mod json_mode;
mod message;
mod start;
mod stop;
mod tests;

use chunk::{Branch, StreamEventChoice, StreamEventChunk, StreamEventDelta};
use error::StreamEventError;

pub struct StreamEventConverter<Counter> {
    pub state: StreamState,
    pub counter: Counter,
    // LiteLLM tracks the buffer's closing byte as fragments arrive rather than rescanning the buffer
    pub json_could_close: bool,
}

impl<Counter> StreamEventConverter<Counter>
where
    Counter: ReasoningTokenCounter,
{
    pub fn chunk_parser(
        &mut self,
        chunk: &Json,
    ) -> Result<StreamEventChunk, StreamEventError<Counter::Failure>> {
        let fields = dispatch::event_fields(chunk)?;
        let mut branch = Branch::default();
        match dispatch::event_type(fields) {
            "content_block_delta" => dispatch::content_block_delta(self, chunk, &mut branch)?,
            "content_block_start" => start::content_block_start(self, chunk, &mut branch)?,
            "content_block_stop" => stop::content_block_stop(self, &mut branch),
            "message_delta" => message::message_delta(self, chunk, &mut branch)?,
            "message_start" => message::message_start(self, chunk, &mut branch)?,
            "error" => {
                return Err(StreamEventError::from(message::native_error(chunk, fields)));
            }
            _ => {}
        }
        json_mode::apply(self, &mut branch)?;
        Ok(self.assemble(branch))
    }

    pub fn handle_accumulated_json_chunk(
        &mut self,
        data: &str,
        is_final: bool,
    ) -> Result<Option<StreamEventChunk>, StreamEventError<Counter::Failure>> {
        self.state.json_buffer.append(data);
        self.json_could_close = accumulated::updated_could_close(self.json_could_close, data);
        if !is_final && !self.json_could_close {
            return Ok(None);
        }
        let Some(value) =
            accumulated::pop_next_value(&mut self.state.json_buffer, &mut self.json_could_close)
        else {
            return Ok(None);
        };
        self.chunk_parser(&value).map(Some)
    }

    fn assemble(&self, branch: Branch) -> StreamEventChunk {
        let provider_specific_fields = (!branch.provider_specific_fields.is_empty())
            .then_some(branch.provider_specific_fields);
        StreamEventChunk {
            id: self.state.response_id.clone(),
            model: self.state.served_model.clone(),
            usage: branch.usage,
            choices: vec![StreamEventChoice {
                index: 0,
                finish_reason: branch.finish_reason,
                delta: StreamEventDelta {
                    content: branch.content,
                    tool_calls: branch.tool_call.map(|call| vec![call]),
                    provider_specific_fields,
                    thinking_blocks: branch.thinking_blocks,
                    reasoning_content: branch.reasoning_content,
                },
            }],
        }
    }
}

// The block-start input stays in place when no argument text arrived or the text is not JSON.
pub fn decode_stream_tool_arguments(arguments: &str) -> Result<Json, error::ShapeError> {
    if arguments.is_empty() {
        return Err(error::ShapeError::NoArguments);
    }
    serde_json::from_str(arguments).map_err(|source| error::ShapeError::Decode { source })
}
