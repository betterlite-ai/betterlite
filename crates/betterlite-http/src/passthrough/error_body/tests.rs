#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::stream;
use futures_util::{Stream, StreamExt as _};
use http::StatusCode;
use http::header::{HeaderMap, HeaderValue};

use super::{
    ChunkStream, PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS, PrefixReplayStream,
    error_body_preview_and_relay, headers_without_body_framing, read_error_body_preview,
};
use crate::error::HttpError;

fn chunks(items: Vec<Result<Vec<u8>, HttpError>>) -> ChunkStream {
    stream::iter(items).boxed()
}

fn body(items: Vec<Vec<u8>>) -> ChunkStream {
    chunks(items.into_iter().map(Ok).collect())
}

async fn collected<S>(stream: S) -> Vec<Result<Vec<u8>, HttpError>>
where
    S: Stream<Item = Result<Vec<u8>, HttpError>>,
{
    stream.collect().await
}

fn delivered(items: Vec<Result<Vec<u8>, HttpError>>) -> Vec<Vec<u8>> {
    items.into_iter().map(|item| item.unwrap()).collect()
}

#[tokio::test]
async fn prefix_replay_stream() {
    let replay = PrefixReplayStream::new(b"err".to_vec(), body(vec![b"a".to_vec(), b"b".to_vec()]));

    assert_eq!(
        delivered(collected(replay).await),
        vec![b"err".to_vec(), b"a".to_vec(), b"b".to_vec()]
    );
}

#[tokio::test]
async fn prefix_replay_stream_without_a_prefix_relays_only_the_rest() {
    let replay = PrefixReplayStream::new(Vec::new(), body(vec![b"a".to_vec()]));

    assert_eq!(delivered(collected(replay).await), vec![b"a".to_vec()]);
}

struct ClosesOnDrop(Arc<AtomicBool>);

impl Drop for ClosesOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn prefix_replay_stream_across_the_close_releases_the_upstream() {
    let closed = Arc::new(AtomicBool::new(false));
    let guard = ClosesOnDrop(Arc::clone(&closed));
    let rest = stream::once(async move {
        drop(guard);
        Ok::<Vec<u8>, HttpError>(Vec::new())
    })
    .boxed();
    let mut replay = PrefixReplayStream::new(b"err".to_vec(), rest);

    replay.aclose();

    assert!(closed.load(Ordering::SeqCst));
    assert!(collected(replay).await.is_empty());
}

#[tokio::test]
async fn passthrough_read_error_body() {
    let first = vec![b'x'; PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS];
    let second = vec![b'y'; 10];
    let tail = b"tail".to_vec();

    let (prefix, rest) = read_error_body_preview(body(vec![first, second, tail.clone()])).await;

    assert_eq!(
        prefix.len(),
        PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS + 10
    );
    assert_eq!(delivered(collected(rest).await), vec![tail]);
}

#[tokio::test]
async fn passthrough_read_error_body_exactly_at_the_budget_reads_to_the_end() {
    let exact = vec![b'x'; PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS];

    let (prefix, rest) = read_error_body_preview(body(vec![exact.clone()])).await;

    assert_eq!(prefix, exact);
    assert!(collected(rest).await.is_empty());
}

#[tokio::test]
async fn passthrough_read_error_body_first_chunk_over_the_budget_is_kept() {
    let big = vec![b'x'; PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS + 1];
    let tail = b"tail".to_vec();

    let (prefix, rest) = read_error_body_preview(body(vec![big.clone(), tail.clone()])).await;

    assert_eq!(prefix, big);
    assert_eq!(delivered(collected(rest).await), vec![tail]);
}

#[tokio::test]
async fn passthrough_read_error_body_read_failure_ends_the_relay() {
    let (prefix, rest) = read_error_body_preview(chunks(vec![
        Ok(b"part".to_vec()),
        Err(HttpError::Timeout),
        Ok(b"never".to_vec()),
    ]))
    .await;

    assert_eq!(prefix, b"part".to_vec());
    assert!(collected(rest).await.is_empty());
}

#[test]
fn passthrough_error_body_preview() {
    let mut headers = HeaderMap::new();
    headers.insert("content-encoding", HeaderValue::from_static("gzip"));
    headers.insert("content-length", HeaderValue::from_static("9"));
    headers.insert("x-up", HeaderValue::from_static("1"));

    let kept = headers_without_body_framing(&headers);

    assert_eq!(kept.len(), 1);
    assert_eq!(kept.get("x-up"), Some(&HeaderValue::from_static("1")));
}

#[tokio::test]
async fn passthrough_error_body_preview_decodes_and_replays() {
    let mut headers = HeaderMap::new();
    headers.insert("content-length", HeaderValue::from_static("9"));

    let relayed = error_body_preview_and_relay(
        StatusCode::BAD_REQUEST,
        &headers,
        body(vec![b"call\xc3\xa9".to_vec(), b"\xff".to_vec()]),
    )
    .await;

    assert_eq!(relayed.preview, "call\u{e9}\u{fffd}");
    assert_eq!(relayed.status, StatusCode::BAD_REQUEST);
    assert!(!relayed.headers.contains_key("content-length"));
    assert_eq!(
        delivered(collected(relayed.stream).await),
        vec![b"call\xc3\xa9\xff".to_vec()]
    );
}

#[tokio::test]
async fn passthrough_error_body_preview_replays_beyond_the_budget() {
    let big = vec![b'x'; PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS + 1];
    let tail = b"tail".to_vec();

    let relayed = error_body_preview_and_relay(
        StatusCode::INTERNAL_SERVER_ERROR,
        &HeaderMap::new(),
        body(vec![big.clone(), tail.clone()]),
    )
    .await;

    assert_eq!(relayed.preview.len(), big.len());
    assert_eq!(delivered(collected(relayed.stream).await), vec![big, tail]);
}

#[tokio::test]
async fn passthrough_error_body_preview_keeps_valid_utf8() {
    let relayed = error_body_preview_and_relay(
        StatusCode::BAD_GATEWAY,
        &HeaderMap::new(),
        body(vec!["\u{e9}\u{20ac}".as_bytes().to_vec()]),
    )
    .await;

    assert_eq!(relayed.preview, "\u{e9}\u{20ac}");
}

#[tokio::test]
async fn passthrough_error_body_preview_replaces_a_truncated_sequence() {
    let relayed = error_body_preview_and_relay(
        StatusCode::BAD_GATEWAY,
        &HeaderMap::new(),
        body(vec![b"ok\xe2\x82".to_vec()]),
    )
    .await;

    assert_eq!(relayed.preview, "ok\u{fffd}");
}
