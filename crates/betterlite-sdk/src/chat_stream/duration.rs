use std::ops::Sub;

use super::ChatStream;
use super::error::ChatStreamCause;

// _check_max_streaming_duration against the wall clock taken in __init__.
pub(super) fn exceeded<S>(stream: &ChatStream<'_, S>) -> Option<ChatStreamCause> {
    let limit = stream.config.max_streaming_duration?;
    let elapsed = Sub::sub(stream.ports.clock.now().timestamp, stream.started_at);
    if elapsed > limit {
        let message =
            format!("Stream exceeded max streaming duration of {limit:?}s (elapsed {elapsed:.1}s)");
        return Some(ChatStreamCause::Duration { message });
    }
    None
}
