#![cfg(test)]

use super::is_raw_sse_stream;
use crate::guardrails::anthropic_sse::tests::support::{
    GOOGLE_FRAME, bytes_chunk, object_chunk, text_chunk,
};

#[test]
fn is_raw_sse_stream_is_true_for_bytes() {
    assert!(is_raw_sse_stream(&[bytes_chunk(b"data: {}\n\n")]));
}

#[test]
fn is_raw_sse_stream_is_true_for_text() {
    assert!(is_raw_sse_stream(&[text_chunk(GOOGLE_FRAME)]));
}

#[test]
fn is_raw_sse_stream_is_true_when_one_chunk_among_objects_is_raw() {
    assert!(is_raw_sse_stream(&[
        object_chunk(),
        object_chunk(),
        bytes_chunk(b"x")
    ]));
}

#[test]
fn is_raw_sse_stream_is_false_for_assembled_chunks_only() {
    assert!(!is_raw_sse_stream(&[object_chunk(), object_chunk()]));
}

#[test]
fn is_raw_sse_stream_is_false_for_no_chunks() {
    assert!(!is_raw_sse_stream(&[]));
}
