#![cfg(test)]

use super::shared::{bytes, chat, coalesce, describe, ok};
use crate::presidio_streams::coalesce::MAX_FIRST_SSE_FRAME_BYTES;

#[test]
fn the_byte_cap_is_64_kib() {
    assert_eq!(MAX_FIRST_SSE_FRAME_BYTES, 0x1_0000);
}

#[tokio::test]
async fn joins_a_frame_split_across_chunks() {
    let events = ok(vec![
        bytes("data: {\"a\""),
        bytes(":1}\n\n"),
        bytes("data: b\n\n"),
    ]);
    assert_eq!(
        coalesce(events).await,
        ["bytes data: {\"a\":1}\n\n", "bytes data: b\n\n"]
    );
}

#[tokio::test]
async fn relays_leading_frames_without_a_data_line() {
    let events = ok(vec![
        bytes(": ka\n\n"),
        bytes("event: x\n\n"),
        bytes("data: a\n\n"),
    ]);
    assert_eq!(
        coalesce(events).await,
        [
            "preface : ka\n\n",
            "preface event: x\n\n",
            "bytes data: a\n\n"
        ]
    );
}

#[tokio::test]
async fn splits_one_chunk_into_preface_first_frame_and_tail() {
    let events = ok(vec![bytes(": ka\n\ndata: a\n\ndata: b"), bytes("c\n\n")]);
    assert_eq!(
        coalesce(events).await,
        [
            "preface : ka\n\n",
            "bytes data: a\n\ndata: b",
            "bytes c\n\n"
        ]
    );
}

#[tokio::test]
async fn relays_non_bytes_events_untouched_while_searching() {
    let events = ok(vec![bytes("data: a"), chat("x"), bytes("\n\n")]);
    assert_eq!(
        coalesce(events).await,
        [describe(&chat("x")), "bytes data: a\n\n".to_owned()]
    );
}

#[tokio::test]
async fn an_unterminated_frame_stops_at_the_byte_cap() {
    let at_cap = "d".repeat(MAX_FIRST_SSE_FRAME_BYTES);
    let lines = coalesce(ok(vec![bytes(&at_cap), bytes("next")])).await;
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines.first().map(String::len),
        Some("bytes ".len() + at_cap.len())
    );
    assert_eq!(lines.get(1).map(String::as_str), Some("bytes next"));
}

#[tokio::test]
async fn a_frame_one_byte_under_the_cap_keeps_joining() {
    let below_cap = "d".repeat(MAX_FIRST_SSE_FRAME_BYTES - 1);
    let lines = coalesce(ok(vec![bytes(&below_cap), bytes("e"), bytes("next")])).await;
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines.first().map(String::len),
        Some("bytes ".len() + MAX_FIRST_SSE_FRAME_BYTES)
    );
}

#[tokio::test]
async fn forwards_everything_after_the_first_frame_untouched() {
    let events = ok(vec![
        bytes("data: a\n\n"),
        bytes(": ka\n\n"),
        bytes("da"),
        chat("y"),
    ]);
    assert_eq!(
        coalesce(events).await,
        [
            "bytes data: a\n\n".to_owned(),
            "bytes : ka\n\n".to_owned(),
            "bytes da".to_owned(),
            describe(&chat("y")),
        ]
    );
}
