#![cfg(test)]

use super::shared::{Events, bytes, coalesce, ok, upstream_failure};

#[tokio::test]
async fn yields_pending_bytes_when_the_stream_ends_without_a_data_frame() {
    let events = ok(vec![bytes(": ka\n\n"), bytes("partial")]);
    assert_eq!(
        coalesce(events).await,
        ["preface : ka\n\n", "bytes partial"]
    );
}

#[tokio::test]
async fn yields_an_unterminated_data_line_at_the_end() {
    assert_eq!(
        coalesce(ok(vec![bytes("data: x")])).await,
        ["bytes data: x"]
    );
}

#[tokio::test]
async fn an_empty_stream_yields_nothing() {
    assert!(coalesce(Vec::new()).await.is_empty());
}

#[tokio::test]
async fn a_stream_of_only_leading_frames_yields_no_trailing_event() {
    assert_eq!(
        coalesce(ok(vec![bytes(": ka\n\n")])).await,
        ["preface : ka\n\n"]
    );
}

#[tokio::test]
async fn an_upstream_error_follows_the_pending_bytes() {
    let events: Events = vec![Ok(bytes("par")), Err(upstream_failure())];
    assert_eq!(coalesce(events).await, ["bytes par", "error"]);
}

#[tokio::test]
async fn an_upstream_error_with_nothing_pending_is_raised_alone() {
    let events: Events = vec![Ok(bytes(": ka\n\n")), Err(upstream_failure())];
    assert_eq!(coalesce(events).await, ["preface : ka\n\n", "error"]);
}

#[tokio::test]
async fn an_upstream_error_after_the_first_frame_is_passed_on() {
    let events: Events = vec![Ok(bytes("data: a\n\n")), Err(upstream_failure())];
    assert_eq!(coalesce(events).await, ["bytes data: a\n\n", "error"]);
}
