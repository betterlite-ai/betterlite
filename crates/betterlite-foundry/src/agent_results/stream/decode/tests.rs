#![cfg(test)]

use std::collections::VecDeque;

use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream::iter;

use crate::agent_results::error::AgentResultError;
use crate::agent_results::stream::{MAX_LINE_BYTES, StreamIdentity};

use super::ChunkState;

const MODEL: &str = "azure_ai/agents/a";

fn identity() -> StreamIdentity {
    StreamIdentity {
        id: String::from("chatcmpl-00000000"),
        created: 1_700_000_000,
    }
}

// the framing fill hands to the stream: complete lines go out, a partial tail stays buffered
#[tokio::test]
async fn a_body_chunk_becomes_lines_and_keeps_the_tail() {
    let mut state = ChunkState::new(
        iter([Ok::<Vec<u8>, HttpError>(b"data: one\ndata: tw".to_vec())]).boxed(),
        MODEL,
        identity(),
    );
    state.fill().await;
    assert_eq!(state.lines, VecDeque::from([String::from("data: one")]));
    assert!(!state.ended);
}

#[tokio::test]
async fn the_end_of_the_body_flushes_the_buffered_tail() {
    let mut state = ChunkState::new(
        iter([Ok::<Vec<u8>, HttpError>(b"data: tw".to_vec())]).boxed(),
        MODEL,
        identity(),
    );
    state.fill().await;
    assert!(state.lines.is_empty());
    assert!(!state.ended);
    state.fill().await;
    assert!(state.ended);
    assert_eq!(state.lines, VecDeque::from([String::from("data: tw")]));
}

#[tokio::test]
async fn a_transport_error_is_kept_as_the_failure() {
    let mut state = ChunkState::new(
        iter([Err::<Vec<u8>, HttpError>(HttpError::Timeout)]).boxed(),
        MODEL,
        identity(),
    );
    state.fill().await;
    assert!(matches!(
        state.failure,
        Some(AgentResultError::Body(HttpError::Timeout))
    ));
    assert!(!state.ended);
}

#[tokio::test]
async fn a_line_past_the_limit_is_kept_as_the_failure() {
    let mut long = b"data: ".to_vec();
    long.resize(MAX_LINE_BYTES.saturating_add(1), b'x');
    let mut state = ChunkState::new(
        iter([Ok::<Vec<u8>, HttpError>(long)]).boxed(),
        MODEL,
        identity(),
    );
    state.fill().await;
    assert!(matches!(
        state.failure,
        Some(AgentResultError::LineTooLong {
            limit: MAX_LINE_BYTES
        })
    ));
    assert!(!state.ended);
}
