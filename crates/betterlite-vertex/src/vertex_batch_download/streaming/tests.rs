#![cfg(test)]

use std::collections::VecDeque;
use std::pin::Pin;
use std::str::from_utf8;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context as TaskContext, Poll};

use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream, Stream};
use http::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use uuid::Uuid;

use crate::vertex_batch_download::records::ConversionContext;
use crate::vertex_batch_download::streaming::{
    DownloadDecoder, DownloadMode, PEEK_LIMIT_BYTES, decode_download_chunk, finish_download,
    transform_file_content_stream,
};

const NOW: i64 = 1_700_000_000;
const URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-2.5-flash%2Fbatch.jsonl";
const PASSTHROUGH_URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpassthrough%2Fbatch.jsonl";
const EMBEDDING_URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-embedding-001%2Fbatch.jsonl";
const ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}}"#;
const OTHER_ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{"candidates":[{"content":{"role":"model","parts":[{"text":"yo"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":2,"totalTokenCount":4}}}"#;
const EMBEDDING_ROWS: &str = concat!(
    r#"{"key":"id%23one#1/2","request":{"content":{}},"status":"","response":{"embedding":{"values":[2]},"usageMetadata":{"promptTokenCount":2}}}"#,
    "\n",
    r#"{"key":"id%23one#0/2","request":{"content":{}},"status":"","response":{"embedding":{"values":[1]},"usageMetadata":{"promptTokenCount":1}}}"#,
);

fn context() -> ConversionContext {
    let mut issued = 0_u128;
    ConversionContext {
        now: NOW,
        ids: Box::new(move || {
            issued = issued.saturating_add(1);
            Uuid::from_u128(issued)
        }),
    }
}

fn decoder(peek_limit: usize) -> DownloadDecoder {
    DownloadDecoder::new(HeaderMap::new(), None, peek_limit, context())
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn source(chunks: &[&[u8]]) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    let items: Vec<Result<Vec<u8>, HttpError>> =
        chunks.iter().map(|part| Ok(part.to_vec())).collect();
    stream::iter(items).boxed()
}

fn drain(mut decoder: DownloadDecoder, chunks: &[&[u8]]) -> (DownloadDecoder, Vec<u8>) {
    let mut output = Vec::new();
    for part in chunks {
        let bytes = decode_download_chunk(&mut decoder, part);
        output.extend_from_slice(&bytes);
    }
    let bytes = finish_download(&mut decoder);
    output.extend_from_slice(&bytes);
    (decoder, output)
}

fn lines(output: &[u8]) -> Vec<&str> {
    from_utf8(output).unwrap().split('\n').collect()
}

fn assert_raw_second_row(row: &str) {
    let content = format!("{ROW}\n{row}\n");
    let (decoder, output) = drain(decoder(PEEK_LIMIT_BYTES), &[content.as_bytes()]);
    assert_eq!(decoder.mode, DownloadMode::Lines);
    assert_eq!(lines(&output)[1], row);
}

async fn assert_untouched(content: &str, url: &str, disabled: bool) {
    let result = transform_file_content_stream(
        source(&[content.as_bytes()]),
        url,
        headers(&[(CONTENT_LENGTH.as_str(), "999")]),
        disabled,
        context(),
    )
    .await
    .unwrap();
    assert_eq!(result.headers.get(CONTENT_LENGTH).unwrap(), "999");
    let mut stream = result.stream;
    let chunk = stream.next().await.unwrap().unwrap();
    assert_eq!(chunk.as_slice(), content.as_bytes());
    assert!(stream.next().await.is_none());
}

#[test]
fn peek_limit_replay() {
    let (replayed, prefix) = drain(decoder(4), &[b"ab", b"cde"]);
    assert_eq!(replayed.mode, DownloadMode::Passthrough);
    assert_eq!(prefix.as_slice(), b"abcde");
    assert!(replayed.headers.get(CONTENT_LENGTH).is_none());

    let mut at_the_limit = decoder(2);
    let output = decode_download_chunk(&mut at_the_limit, b"{}");
    assert!(output.is_empty());
    assert_eq!(at_the_limit.mode, DownloadMode::Undecided);
}

#[test]
fn newline_before_limit() {
    let content = format!("{ROW}\n");
    let mut decoder = decoder(2);
    let output = decode_download_chunk(&mut decoder, content.as_bytes());
    assert!(content.len() > 2);
    assert_eq!(decoder.mode, DownloadMode::Lines);
    assert_eq!(lines(&output).len(), 1);
    assert!(lines(&output)[0].starts_with(r#"{"id":"batch_req_"#));
}

#[test]
fn split_utf8() {
    let row = ROW.replace(r#""hi""#, "\"\u{e9}\"");
    let position = row.find('\u{e9}').unwrap().saturating_add(1);
    let (head, tail) = row.as_bytes().split_at(position);
    let mut completed = tail.to_vec();
    completed.push(b'\n');
    let (decoder, output) = drain(decoder(PEEK_LIMIT_BYTES), &[head, completed.as_slice()]);
    assert_eq!(decoder.mode, DownloadMode::Lines);
    assert!(from_utf8(&output).unwrap().contains('\u{e9}'));
}

#[test]
fn malformed_middle_row() {
    let content = format!("{ROW}\nnot json\n{OTHER_ROW}\n");
    let mut decoder = decoder(PEEK_LIMIT_BYTES);
    let output = decode_download_chunk(&mut decoder, content.as_bytes());
    assert_eq!(decoder.mode, DownloadMode::Lines);
    let lines = lines(&output);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1], "not json");
    assert!(lines[0].starts_with(r#"{"id":"batch_req_00000000-0000-0000-0000-000000000002""#));
    assert!(lines[2].starts_with(r#"{"id":"batch_req_00000000-0000-0000-0000-000000000004""#));
}

#[test]
fn case_insensitive_length_removal() {
    let mut decoder = DownloadDecoder::new(
        headers(&[
            ("Content-Length", "123"),
            ("Content-Type", "application/json"),
        ]),
        None,
        PEEK_LIMIT_BYTES,
        context(),
    );
    let content = format!("{ROW}\n");
    let output = decode_download_chunk(&mut decoder, content.as_bytes());
    assert_eq!(decoder.mode, DownloadMode::Lines);
    assert!(decoder.headers.get(CONTENT_LENGTH).is_none());
    assert_eq!(
        decoder.headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert!(!output.is_empty());
}

#[tokio::test]
async fn a_blank_download_passes_through() {
    let content: &[u8] = b"\n\n";
    let result = transform_file_content_stream(
        source(&[content]),
        URL,
        headers(&[(CONTENT_LENGTH.as_str(), "2")]),
        false,
        context(),
    )
    .await
    .unwrap();
    assert_eq!(result.headers.get(CONTENT_LENGTH).unwrap(), "2");
    let mut stream = result.stream;
    assert_eq!(stream.next().await.unwrap().unwrap().as_slice(), content);
    assert!(stream.next().await.is_none());
}

#[test]
fn an_unknown_first_row_replays_the_prefix() {
    let content = format!("{{\"custom_id\":\"x\"}}\n{ROW}\n");
    let (decoder, output) = drain(decoder(PEEK_LIMIT_BYTES), &[content.as_bytes()]);
    assert_eq!(decoder.mode, DownloadMode::Passthrough);
    assert_eq!(output.as_slice(), content.as_bytes());
    assert!(decoder.headers.get(CONTENT_LENGTH).is_none());
}

#[test]
fn blank_lines_never_decide() {
    let completed = format!("{ROW}\n");
    let (decoder, output) = drain(decoder(4), &[b"\n\n", completed.as_bytes()]);
    assert_eq!(decoder.mode, DownloadMode::Lines);
    assert_eq!(lines(&output).len(), 1);
    assert!(lines(&output)[0].starts_with(r#"{"id":"batch_req_"#));
}

// trim_ascii follows the WhatWG set and leaves a vertical tab, which Python's bytes.strip drops
#[test]
fn a_vertical_tab_prefix_replays_the_download() {
    let content = format!("\x0b{ROW}\n");
    let (decoder, output) = drain(decoder(PEEK_LIMIT_BYTES), &[content.as_bytes()]);
    assert_eq!(decoder.mode, DownloadMode::Passthrough);
    assert_eq!(output.as_slice(), content.as_bytes());
}

// bytes.strip drops a form feed, so a padded first row still selects the converted mode
#[test]
fn a_form_feed_padded_first_row_is_converted() {
    let content = format!("\x0c{ROW}\x0c\n");
    let (decoder, output) = drain(decoder(PEEK_LIMIT_BYTES), &[content.as_bytes()]);
    assert_eq!(decoder.mode, DownloadMode::Lines);
    let lines = lines(&output);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with(r#"{"id":"batch_req_"#));
}

#[test]
fn a_trailing_row_without_a_newline_is_converted() {
    let (decoder, output) = drain(decoder(PEEK_LIMIT_BYTES), &[ROW.as_bytes()]);
    assert_eq!(decoder.mode, DownloadMode::Lines);
    assert!(
        from_utf8(&output)
            .unwrap()
            .starts_with(r#"{"id":"batch_req_"#)
    );
}

#[test]
fn rows_are_newline_joined() {
    let content = format!("{ROW}\n{OTHER_ROW}\n");
    let mut decoder = decoder(PEEK_LIMIT_BYTES);
    let output = decode_download_chunk(&mut decoder, content.as_bytes());
    assert_eq!(decoder.mode, DownloadMode::Lines);
    let lines = lines(&output);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with(r#"{"id":"batch_req_00000000-0000-0000-0000-000000000002""#));
    assert!(lines[1].starts_with(r#"{"id":"batch_req_00000000-0000-0000-0000-000000000004""#));
}

#[test]
fn a_failing_row_keeps_its_bytes() {
    let labels = ROW.replace(r#""request":{}"#, r#""request":{"labels":"nope"}"#);
    assert_raw_second_row(&labels);
    let response = ROW.replace(
        r#""response":{"#,
        r#""response":"not an object","broken":{"#,
    );
    assert_raw_second_row(&response);
    let version = ROW.replace(r#""candidates""#, r#""modelVersion":5,"candidates""#);
    assert_raw_second_row(&version);
    let null_version = ROW.replace(r#""candidates""#, r#""modelVersion":null,"candidates""#);
    assert_raw_second_row(&null_version);
}

#[tokio::test]
async fn closes_upstream_on_drop() {
    let tracked = Tracked::new(vec![ROW.as_bytes().to_vec()]);
    let dropped = Arc::clone(&tracked.dropped);
    let polls = Arc::clone(&tracked.polls);
    let result =
        transform_file_content_stream(tracked.boxed(), URL, HeaderMap::new(), false, context())
            .await
            .unwrap();
    assert!(!dropped.load(Ordering::SeqCst));
    assert_eq!(polls.load(Ordering::SeqCst), 2);
    let mut stream = result.stream;
    let chunk = stream.next().await.unwrap().unwrap();
    assert!(
        from_utf8(&chunk)
            .unwrap()
            .starts_with(r#"{"id":"batch_req_"#)
    );
    assert!(stream.next().await.is_none());
    drop(stream);
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_newline_terminated_download_ends_without_an_empty_chunk() {
    let content = format!("{ROW}\n");
    let result = transform_file_content_stream(
        source(&[content.as_bytes()]),
        URL,
        HeaderMap::new(),
        false,
        context(),
    )
    .await
    .unwrap();
    let mut stream = result.stream;
    assert!(
        stream
            .next()
            .await
            .unwrap()
            .unwrap()
            .starts_with(b"{\"id\":\"batch_req_")
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn cancelling_the_stream_closes_the_source() {
    let content = format!("{ROW}\n{OTHER_ROW}\n");
    let (head, tail) = content.split_once('\n').unwrap();
    let tracked = Tracked::new(vec![
        format!("{head}\n").into_bytes(),
        tail.as_bytes().to_vec(),
    ]);
    let dropped = Arc::clone(&tracked.dropped);
    let polls = Arc::clone(&tracked.polls);
    let result =
        transform_file_content_stream(tracked.boxed(), URL, HeaderMap::new(), false, context())
            .await
            .unwrap();
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert!(!dropped.load(Ordering::SeqCst));
    let mut stream = result.stream;
    let chunk = stream.next().await.unwrap().unwrap();
    assert!(
        from_utf8(&chunk)
            .unwrap()
            .starts_with(r#"{"id":"batch_req_"#)
    );
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    drop(stream);
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn later_rows_stream_in_as_their_chunks_complete() {
    let third_row = ROW.replace(r#""hi""#, r#""za""#);
    let first_chunk = format!("{ROW}\n");
    let second_chunk = format!("{OTHER_ROW}\n");
    let result = transform_file_content_stream(
        source(&[
            first_chunk.as_bytes(),
            second_chunk.as_bytes(),
            third_row.as_bytes(),
        ]),
        URL,
        HeaderMap::new(),
        false,
        context(),
    )
    .await
    .unwrap();
    let mut stream = result.stream;
    let first = stream.next().await.unwrap().unwrap();
    assert!(first.starts_with(b"{\"id\":\"batch_req_"));
    let second = stream.next().await.unwrap().unwrap();
    assert!(second.starts_with(b"\n{\"id\":\"batch_req_"));
    assert!(from_utf8(&second).unwrap().contains(r#""content":"yo""#));
    let third = stream.next().await.unwrap().unwrap();
    assert!(third.starts_with(b"\n{\"id\":\"batch_req_"));
    assert!(from_utf8(&third).unwrap().contains(r#""content":"za""#));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn streamed_embeddings_answer_in_one_chunk() {
    let result = transform_file_content_stream(
        source(&[EMBEDDING_ROWS.as_bytes()]),
        EMBEDDING_URL,
        headers(&[(CONTENT_LENGTH.as_str(), "1")]),
        false,
        context(),
    )
    .await
    .unwrap();
    let declared = result
        .headers
        .get(CONTENT_LENGTH)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let mut stream = result.stream;
    let chunk = stream.next().await.unwrap().unwrap();
    assert!(stream.next().await.is_none());
    assert_eq!(declared, chunk.len().to_string());
    assert!(
        from_utf8(&chunk)
            .unwrap()
            .contains(r#""model":"gemini-embedding-001""#)
    );
}

#[tokio::test]
async fn embedding_fallback_keeps_the_bytes_and_sets_the_length() {
    let content = format!("{EMBEDDING_ROWS}\nnot json\n");
    let result = transform_file_content_stream(
        source(&[content.as_bytes()]),
        EMBEDDING_URL,
        headers(&[(CONTENT_LENGTH.as_str(), "1")]),
        false,
        context(),
    )
    .await
    .unwrap();
    let declared = result
        .headers
        .get(CONTENT_LENGTH)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(declared, content.len().to_string());
    let mut stream = result.stream;
    let chunk = stream.next().await.unwrap().unwrap();
    assert_eq!(chunk.as_slice(), content.as_bytes());
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn disabled_and_passthrough_streams_keep_every_byte() {
    let content = format!("{ROW}\n");
    assert_untouched(&content, URL, true).await;
    assert_untouched(&content, PASSTHROUGH_URL, false).await;
}

struct Tracked {
    chunks: VecDeque<Vec<u8>>,
    dropped: Arc<AtomicBool>,
    polls: Arc<AtomicUsize>,
}

impl Tracked {
    fn new(chunks: Vec<Vec<u8>>) -> Self {
        Self {
            chunks: VecDeque::from(chunks),
            dropped: Arc::new(AtomicBool::new(false)),
            polls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl Stream for Tracked {
    type Item = Result<Vec<u8>, HttpError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        _context: &mut TaskContext<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(self.chunks.pop_front().map(Ok))
    }
}
