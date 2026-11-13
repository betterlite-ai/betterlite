#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use rstest::rstest;
use uuid::Uuid;

use crate::guardrails::chat_guardrail_translation::block_framing::blocked_identity::{
    FreshIdentity, StreamIdentity, blocked_stream_identity,
};
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, stream_items};

const FRESH: FreshIdentity = FreshIdentity {
    id: Uuid::from_u128(0x1234),
    created: 99,
};

fn identity(id: &str, created: i64, model: &str) -> StreamIdentity {
    StreamIdentity {
        id: id.to_owned(),
        created,
        model: model.to_owned(),
    }
}

fn fresh_id() -> String {
    format!("chatcmpl-{}", FRESH.id)
}

fn resolve(chunks: Vec<JsonValue>) -> StreamIdentity {
    let items = stream_items(chunks);
    blocked_stream_identity("fallback", &items, &FRESH)
}

#[rstest]
#[case::all_three_from_the_chunk(
    vec![json(r#"{"id": "c1", "created": 5, "model": "m1"}"#)],
    identity("c1", 5, "m1"),
)]
#[case::first_chunk_with_an_id_wins(
    vec![json(r#"{"id": "c1", "created": 5, "model": "m1"}"#), json(r#"{"id": "c2", "created": 6, "model": "m2"}"#)],
    identity("c1", 5, "m1"),
)]
#[case::chunks_without_a_usable_id_are_skipped(
    vec![json(r#"{"created": 1, "model": "x"}"#), json(r#"{"id": "", "created": 2}"#), json(r#"{"id": null}"#), json(r#"{"id": 3}"#), json(r#"{"id": "c9", "created": 5, "model": "m1"}"#)],
    identity("c9", 5, "m1"),
)]
#[case::created_falls_back_alone(
    vec![json(r#"{"id": "c1", "created": "5", "model": "m1"}"#)],
    identity("c1", 99, "m1"),
)]
#[case::missing_created(vec![json(r#"{"id": "c1", "model": "m1"}"#)], identity("c1", 99, "m1"))]
#[case::model_falls_back_alone(vec![json(r#"{"id": "c1", "created": 5, "model": ""}"#)], identity("c1", 5, "fallback"))]
#[case::null_model(vec![json(r#"{"id": "c1", "created": 5, "model": null}"#)], identity("c1", 5, "fallback"))]
#[case::model_of_another_chunk_is_not_borrowed(
    vec![json(r#"{"id": "c1", "created": 5}"#), json(r#"{"id": "c2", "created": 6, "model": "m2"}"#)],
    identity("c1", 5, "fallback"),
)]
fn the_first_identified_chunk_supplies_the_identity(
    #[case] items: Vec<JsonValue>,
    #[case] expected: StreamIdentity,
) {
    assert_eq!(resolve(items), expected);
}

#[rstest]
#[case::no_chunks(vec![])]
#[case::no_chunk_has_an_id(vec![json(r#"{"created": 1, "model": "m1"}"#), json(r#"{"id": ""}"#)])]
fn without_an_id_everything_is_fresh(#[case] items: Vec<JsonValue>) {
    assert_eq!(resolve(items), identity(&fresh_id(), 99, "fallback"));
}

#[test]
fn raw_byte_chunks_carry_no_identity() {
    let items = [StreamItem::Bytes(br#"data: {"id":"c1"}"#.to_vec())];
    assert_eq!(
        blocked_stream_identity("fallback", &items, &FRESH),
        identity(&fresh_id(), 99, "fallback"),
    );
}
