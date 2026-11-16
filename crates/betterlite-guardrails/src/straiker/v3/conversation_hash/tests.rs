#![cfg(test)]

use rstest::rstest;
use sha2::{Digest as _, Sha256};

use super::conversation_prefixes;
use crate::straiker::v3::fixtures::object;

fn after_each(messages: &[&str]) -> Vec<String> {
    let mut digest = Sha256::new();
    messages
        .iter()
        .map(|message| {
            digest.update(message.as_bytes());
            digest.update([0x1e]);
            hex::encode(digest.clone().finalize())
        })
        .collect()
}

fn whole(text: &str) -> Vec<String> {
    vec![hex::encode(Sha256::digest(text.as_bytes()))]
}

#[rstest]
#[case::one_hash_per_message_in_order(
    r#"{"messages":[{"role":"system","content":"s"},{"role":"user","content":"u"}]}"#,
    after_each(&[r#"{"content":"s","role":"system"}"#, r#"{"content":"u","role":"user"}"#])
)]
#[case::keys_are_sorted_at_every_level(
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a"}]}]}"#,
    after_each(&[r#"{"content":[{"text":"a","type":"text"}],"role":"user"}"#])
)]
#[case::key_order_does_not_matter(
    r#"{"messages":[{"content":"u","role":"user"}]}"#,
    after_each(&[r#"{"content":"u","role":"user"}"#])
)]
#[case::responses_input_messages(
    r#"{"input":[{"role":"user","content":"u"}]}"#,
    after_each(&[r#"{"content":"u","role":"user"}"#])
)]
#[case::empty_messages_fall_to_input(r#"{"messages":[],"input":"x"}"#, whole(r#""x""#))]
#[case::non_object_messages_are_skipped(
    r#"{"messages":["x",{"role":"user","content":"u"}]}"#,
    after_each(&[r#"{"content":"u","role":"user"}"#])
)]
#[case::plain_input(r#"{"input":"hello"}"#, whole(r#""hello""#))]
#[case::plain_prompt(r#"{"prompt":"hello"}"#, whole(r#""hello""#))]
#[case::input_wins_over_prompt(r#"{"input":"in","prompt":"pr"}"#, whole(r#""in""#))]
#[case::null_input_hides_prompt(r#"{"input":null,"prompt":"pr"}"#, vec![])]
#[case::prompt_list_is_hashed_whole(r#"{"prompt":["a","b"]}"#, whole(r#"["a","b"]"#))]
#[case::input_list_of_non_objects_is_hashed_whole(r#"{"input":["a",1]}"#, whole(r#"["a",1]"#))]
#[case::nothing_to_hash(r#"{"model":"m"}"#, vec![])]
fn fingerprints_conversation(#[case] source: &str, #[case] expected: Vec<String>) {
    assert_eq!(conversation_prefixes(&object(source)).unwrap(), expected);
}

#[test]
fn prefixes_differ_by_history() {
    let short =
        conversation_prefixes(&object(r#"{"messages":[{"role":"user","content":"a"}]}"#)).unwrap();
    let long = conversation_prefixes(&object(
        r#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"}]}"#,
    ))
    .unwrap();
    assert_eq!(long.first(), short.first());
    assert_eq!(long.len(), 2);
    assert_ne!(long.first(), long.last());
}
