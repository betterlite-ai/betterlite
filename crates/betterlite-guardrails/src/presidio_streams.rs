use serde_json::value::RawValue;
use tokio::sync::mpsc::Sender;

use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::HookContext;

pub mod assembly;
pub mod coalesce;
pub mod error;
pub mod hook;
pub mod mask;
pub mod mask_anthropic;
pub mod unmask;
pub mod unmask_sse;

// What LiteLLM's hook sees one item of the stream as: a parsed chat chunk, a raw SSE frame, or any other event such as a Responses API one
#[derive(Debug, Clone)]
pub enum StreamEvent {
    Chat(Box<RawValue>),
    Bytes(Vec<u8>),
    Other(Box<RawValue>),
}

// Everything a streaming hook run reads and writes besides the stream itself
pub struct StreamRun<'run, Assembly> {
    pub context: HookContext<'run>,
    pub request_data: &'run mut RequestData,
    pub assembly: &'run Assembly,
    pub out: &'run Sender<StreamEvent>,
}

#[path = "presidio_streams/tests/coalesce_first_frame.rs"]
mod coalesce_first_frame;
#[path = "presidio_streams/tests/coalesce_stream_end.rs"]
mod coalesce_stream_end;
#[path = "presidio_streams/tests/flush_unmaskable.rs"]
mod flush_unmaskable;
#[path = "presidio_streams/tests/hook_apply_to_output.rs"]
mod hook_apply_to_output;
#[path = "presidio_streams/tests/hook_output_parse.rs"]
mod hook_output_parse;
#[path = "presidio_streams/tests/hook_passthrough.rs"]
mod hook_passthrough;
#[path = "presidio_streams/tests/mask_anthropic_convert.rs"]
mod mask_anthropic_convert;
#[path = "presidio_streams/tests/mask_anthropic.rs"]
mod mask_anthropic_tests;
#[path = "presidio_streams/tests/mask_buffered.rs"]
mod mask_buffered;
#[path = "presidio_streams/tests/mask_buffered_invalid.rs"]
mod mask_buffered_invalid;
#[path = "presidio_streams/tests/preserve_usage.rs"]
mod preserve_usage;
#[path = "presidio_streams/tests/shared.rs"]
mod shared;
#[path = "presidio_streams/tests/split_sse_preface.rs"]
mod split_sse_preface;
#[path = "presidio_streams/tests/stream_apply_blocked_raise.rs"]
mod stream_apply_blocked_raise;
#[path = "presidio_streams/tests/stream_apply_flush.rs"]
mod stream_apply_flush;
#[path = "presidio_streams/tests/stream_apply_mixed.rs"]
mod stream_apply_mixed;
#[path = "presidio_streams/tests/stream_apply_shape.rs"]
mod stream_apply_shape;
#[path = "presidio_streams/tests/stream_pii_assemble.rs"]
mod stream_pii_assemble;
#[path = "presidio_streams/tests/stream_pii_dispatch.rs"]
mod stream_pii_dispatch;
#[path = "presidio_streams/tests/stream_pii_exception.rs"]
mod stream_pii_exception;
#[path = "presidio_streams/tests/stream_pii_mixed.rs"]
mod stream_pii_mixed;
#[path = "presidio_streams/tests/stream_pii_process.rs"]
mod stream_pii_process;
#[path = "presidio_streams/tests/stream_pii_unmask.rs"]
mod stream_pii_unmask;
#[path = "presidio_streams/tests/stream_pii_usage.rs"]
mod stream_pii_usage;
#[path = "presidio_streams/tests/unmask_responses_api.rs"]
mod unmask_responses_api;
#[path = "presidio_streams/tests/unmask_sse_delta.rs"]
mod unmask_sse_delta;
#[path = "presidio_streams/tests/unmask_sse_errors.rs"]
mod unmask_sse_errors;
