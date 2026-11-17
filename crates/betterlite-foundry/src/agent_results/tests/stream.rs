#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream::iter;

use crate::agent_results::error::AgentResultError;
use crate::agent_results::stream::{MAX_ERROR_BODY_BYTES, MAX_LINE_BYTES, STATUS_OK, open_stream};

use super::support::{
    block_on, body_stream, delta_chunk, object, setup, stream_for_chunks, terminal_chunk, watched,
};

const DONE: &[u8] = b"data: [DONE]\n\n";

fn delta_body(text: &str) -> Vec<u8> {
    format!("event: thread.message.delta\ndata: {{\"delta\":{{\"content\":[{{\"type\":\"text\",\"text\":{{\"value\":\"{text}\"}}}}]}}}}\n\n")
        .into_bytes()
}

#[test]
fn the_stream_starts_lazily_and_drops_the_body_with_it() {
    block_on(async {
        let polls = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let body = watched(vec![delta_body("one"), DONE.to_vec()], &polls, &dropped);
        let mut stream = open_stream(setup(STATUS_OK), body).await.expect("a stream");
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        assert!(!dropped.load(Ordering::SeqCst));

        let first = stream.next().await.expect("a chunk").expect("a chunk");
        assert_eq!(first, delta_chunk("one"));
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert!(!dropped.load(Ordering::SeqCst));

        drop(stream);
        assert!(dropped.load(Ordering::SeqCst));
    });
}

#[test]
fn done_releases_the_body_before_the_stream_ends() {
    block_on(async {
        let polls = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let body = watched(vec![DONE.to_vec(), delta_body("after")], &polls, &dropped);
        let mut stream = open_stream(setup(STATUS_OK), body).await.expect("a stream");

        let terminal = stream.next().await.expect("a chunk").expect("a chunk");
        assert_eq!(terminal, terminal_chunk(None, None));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert!(stream.next().await.is_none());
    });
}

#[test]
fn a_transport_error_is_reported_once_and_ends_the_stream() {
    block_on(async {
        let body = iter([Err::<Vec<u8>, HttpError>(HttpError::Timeout)]).boxed();
        let mut stream = open_stream(setup(STATUS_OK), body).await.expect("a stream");

        let error = stream
            .next()
            .await
            .expect("a failure")
            .expect_err("an error");
        assert!(matches!(error, AgentResultError::Body(HttpError::Timeout)));
        assert_eq!(
            error.to_string(),
            "the streaming response body could not be read"
        );
        assert_eq!(error.status(), None);
        assert!(stream.next().await.is_none());
    });
}

#[test]
fn a_line_at_the_limit_is_kept_and_one_past_it_fails() {
    block_on(async {
        let mut accepted = b"data: ".to_vec();
        accepted.resize(MAX_LINE_BYTES, b'x');
        assert!(
            stream_for_chunks(vec![accepted])
                .await
                .next()
                .await
                .is_none()
        );

        let mut rejected = b"data: ".to_vec();
        rejected.resize(MAX_LINE_BYTES.saturating_add(1), b'x');
        let mut stream = stream_for_chunks(vec![rejected]).await;
        let error = stream
            .next()
            .await
            .expect("a failure")
            .expect_err("an error");
        assert_eq!(
            error.to_string(),
            format!("a streaming response line is longer than {MAX_LINE_BYTES} bytes")
        );
        assert_eq!(error.status(), None);
        assert!(stream.next().await.is_none());
    });
}

#[test]
fn an_error_body_at_the_limit_is_read_and_one_past_it_fails() {
    block_on(async {
        let at_limit = vec![b'x'; MAX_ERROR_BODY_BYTES];
        let read = open_stream(setup(500), body_stream(vec![at_limit]))
            .await
            .err()
            .expect("an error");
        assert_eq!(
            read.to_string().len(),
            "Streaming request failed: "
                .len()
                .saturating_add(MAX_ERROR_BODY_BYTES)
        );
        assert_eq!(read.status(), Some(500));

        let past_limit = vec![b'x'; MAX_ERROR_BODY_BYTES.saturating_add(1)];
        let oversized = open_stream(setup(500), body_stream(vec![past_limit]))
            .await
            .err()
            .expect("an error");
        assert_eq!(
            oversized.to_string(),
            format!("the streaming response body is larger than {MAX_ERROR_BODY_BYTES} bytes")
        );
        assert_eq!(oversized.status(), Some(500));
    });
}

#[test]
fn the_chunk_json_shape_is_pinned() {
    assert_eq!(
        serde_json::to_string(&delta_chunk("hi")).expect("a chunk"),
        concat!(
            r#"{"id":"chatcmpl-00000000","created":1700000000,"model":"azure_ai/agents/a","#,
            r#""object":"chat.completion.chunk","choices":[{"index":0,"finish_reason":null,"#,
            r#""delta":{"content":"hi","role":"assistant","annotations":null}}],"#,
            r#""thread_id":null}"#,
        )
    );
    let annotations = vec![object(r#"{"type": "file_citation", "file_id": "f1"}"#)];
    assert_eq!(
        serde_json::to_string(&terminal_chunk(Some(annotations), Some(String::from("t1"))))
            .expect("a chunk"),
        concat!(
            r#"{"id":"chatcmpl-00000000","created":1700000000,"model":"azure_ai/agents/a","#,
            r#""object":"chat.completion.chunk","choices":[{"index":0,"finish_reason":"stop","#,
            r#""delta":{"content":null,"role":null,"annotations":["#,
            r#"{"type":"file_citation","file_id":"f1"}]}}],"thread_id":"t1"}"#,
        )
    );
}
