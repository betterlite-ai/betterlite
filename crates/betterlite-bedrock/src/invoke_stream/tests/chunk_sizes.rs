use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;

use crate::agentcore_stream::transport::{Chunks, StreamHead};
use crate::bedrock_http_stream::PendingOpen;
use crate::invoke_stream::chunking::apply;

use super::support::block_on;

#[test]
fn chunk_size_cases() {
    splits_reads_at_the_size();
    fills_from_the_next_read();
    flushes_the_tail();
    keeps_transport_reads_without_a_size();
    keeps_transport_reads_for_zero();
    keeps_a_full_read_and_drops_the_tail_at_a_failure();
}

fn pending(reads: Vec<Result<Vec<u8>, HttpError>>) -> PendingOpen {
    PendingOpen {
        head: StreamHead {
            status: 200,
            headers: HeaderMap::new(),
            chunks: stream::iter(reads).boxed(),
        },
        body: None,
    }
}

fn read(mut chunks: Chunks) -> Vec<Result<Vec<u8>, String>> {
    let mut items = Vec::new();
    while let Some(item) = block_on(chunks.next()) {
        items.push(item.map_err(|error| error.to_string()));
    }
    items
}

fn splits_reads_at_the_size() {
    let chunks = apply(
        pending(vec![Ok(b"abc".to_vec()), Ok(b"defgh".to_vec())]),
        Some(4),
    )
    .head
    .chunks;
    assert_eq!(
        read(chunks),
        vec![Ok(b"abcd".to_vec()), Ok(b"efgh".to_vec())]
    );
}

fn fills_from_the_next_read() {
    let chunks = apply(pending(vec![Ok(b"abcdef".to_vec())]), Some(4))
        .head
        .chunks;
    assert_eq!(read(chunks), vec![Ok(b"abcd".to_vec()), Ok(b"ef".to_vec())]);
}

fn flushes_the_tail() {
    let chunks = apply(
        pending(vec![Ok(b"ab".to_vec()), Ok(b"c".to_vec())]),
        Some(4),
    )
    .head
    .chunks;
    assert_eq!(read(chunks), vec![Ok(b"abc".to_vec())]);
}

fn keeps_transport_reads_without_a_size() {
    let chunks = apply(pending(vec![Ok(b"ab".to_vec()), Ok(b"c".to_vec())]), None)
        .head
        .chunks;
    assert_eq!(read(chunks), vec![Ok(b"ab".to_vec()), Ok(b"c".to_vec())]);
}

fn keeps_transport_reads_for_zero() {
    let chunks = apply(pending(vec![Ok(b"ab".to_vec())]), Some(0))
        .head
        .chunks;
    assert_eq!(read(chunks), vec![Ok(b"ab".to_vec())]);
}

// A full read is handed over before the chunker asks the body for more, so the failure never eats it; the bytes it buffered do not follow.
fn keeps_a_full_read_and_drops_the_tail_at_a_failure() {
    let timeout = Err("the request timed out".to_owned());
    let full = apply(
        pending(vec![Ok(b"abcd".to_vec()), Err(HttpError::Timeout)]),
        Some(4),
    )
    .head
    .chunks;
    assert_eq!(read(full), vec![Ok(b"abcd".to_vec()), timeout.clone()]);
    let partial = apply(
        pending(vec![Ok(b"abcde".to_vec()), Err(HttpError::Timeout)]),
        Some(4),
    )
    .head
    .chunks;
    assert_eq!(read(partial), vec![Ok(b"abcd".to_vec()), timeout]);
}
