#![cfg(test)]

use super::super::{is_message_stop_chunk, is_provider_error_chunk};

const STOP: &[u8] = b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
const ERROR: &[u8] = b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"busy\"}}\n\n";
const DELTA: &[u8] = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n";

#[test]
fn frame_probes_classify_stop_and_error() {
    assert!(is_message_stop_chunk(STOP));
    assert!(!is_message_stop_chunk(ERROR));
    assert!(!is_message_stop_chunk(DELTA));
    // the event line decides, not the data JSON's type
    assert!(!is_message_stop_chunk(
        b"data: {\"type\":\"message_stop\"}\n\n"
    ));
    assert!(is_provider_error_chunk(ERROR));
    assert!(!is_provider_error_chunk(STOP));
    assert!(!is_provider_error_chunk(DELTA));
    // a type-only error payload carries no error body
    assert!(!is_provider_error_chunk(
        b"event: error\ndata: {\"type\":\"error\"}\n\n"
    ));
    // a joined stream larger than the per-read bound still classifies
    let mut large = STOP.to_vec();
    large.resize(1024 * 1024 + 1, b'\n');
    assert!(is_message_stop_chunk(&large));
}
