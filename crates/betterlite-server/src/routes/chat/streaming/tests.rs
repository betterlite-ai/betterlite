#![cfg(test)]

use std::collections::VecDeque;

use betterlite_http::error::HttpError;
use betterlite_openai::stream::SseParser;
use futures_util::{StreamExt as _, stream};
use rstest::rstest;

use super::{RelayState, next_sse};

#[rstest]
#[case::transport(Err(HttpError::Timeout))]
#[case::invalid_json(Ok(b"data: invalid\n\n".to_vec()))]
#[case::invalid_utf8(Ok(b"data: \xff\n\n".to_vec()))]
#[case::oversized(Ok(vec![b'x'; 0x0010_0001]))]
#[tokio::test]
async fn stream_error_ends_relay(#[case] input: Result<Vec<u8>, HttpError>) {
    let state = RelayState {
        upstream: stream::iter(vec![input, Ok(b"data: [DONE]\n\n".to_vec())]).boxed(),
        parser: SseParser::default(),
        pending: VecDeque::new(),
        finished: false,
        model: String::from("group"),
        include_usage: false,
    };
    let (event, remaining) = next_sse(state).await.unwrap();
    event.unwrap_err();
    assert!(next_sse(remaining).await.is_none());
}
