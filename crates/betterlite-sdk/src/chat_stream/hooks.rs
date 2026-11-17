use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use crate::call_setup::hooks::hook_call_type;

use super::ports::ChatStreamPorts;

// _call_post_streaming_deployment_hook: a hook replaces the chunk, and a broken hook only loses itself.
pub(super) async fn run(ports: &ChatStreamPorts<'_>, mut chunk: StreamChunk) -> StreamChunk {
    let call_type = hook_call_type(ports.call_type);
    for hook in ports.hooks {
        let outcome = hook
            .stream_chunk(ports.request_data, call_type, &chunk)
            .await;
        match outcome {
            Ok(Some(replacement)) => chunk = replacement,
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "Error in post-call streaming deployment hook");
                return chunk;
            }
        }
    }
    chunk
}
