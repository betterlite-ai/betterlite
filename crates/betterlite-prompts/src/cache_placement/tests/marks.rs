use rstest::rstest;

use crate::arguments::json::Json;
use crate::cache_placement::marks::{
    count_message_marks, count_request_marks, count_system_marks, fields_carry_mark,
    system_has_mark,
};
use crate::cache_placement::system::SystemPrompt;
use crate::cache_placement::tests::support::{json, messages};

fn system_of(wire: &str) -> SystemPrompt {
    serde_json::from_str(wire).unwrap()
}

#[rstest]
#[case::nothing("{}", false)]
#[case::cache_control(r#"{"cache_control": {"type": "ephemeral"}}"#, true)]
#[case::breakpoint(r#"{"prompt_cache_breakpoint": {"mode": "explicit"}}"#, true)]
#[case::null_control(r#"{"cache_control": null}"#, false)]
#[case::null_breakpoint(r#"{"prompt_cache_breakpoint": null}"#, false)]
#[case::unrelated(r#"{"name": "x"}"#, false)]
fn a_field_carries_a_mark_only_when_the_key_holds_a_value(
    #[case] wire: &str,
    #[case] expected: bool,
) {
    let Json::Object(fields) = json(wire) else {
        panic!("expected an object")
    };
    assert_eq!(fields_carry_mark(&fields), expected);
}

#[rstest]
#[case::plain(r#"[{"role": "user", "content": "hi"}]"#, 0)]
#[case::top_level(r#"[{"role": "user", "content": "hi", "cache_control": {}}]"#, 1)]
#[case::one_block(
    r#"[{"role": "user", "content": [{"type": "text", "cache_control": {}}]}]"#,
    1
)]
#[case::two_blocks(r#"[{"role": "user", "content": [{"type": "text", "cache_control": {}}, {"type": "image", "prompt_cache_breakpoint": {}}]}]"#, 2)]
#[case::top_and_block(r#"[{"role": "user", "content": [{"type": "text", "cache_control": {}}], "cache_control": {}}]"#, 2)]
#[case::null_does_not_count(
    r#"[{"role": "user", "content": [{"type": "text", "cache_control": null}]}]"#,
    0
)]
fn a_message_counts_its_top_level_mark_and_its_marked_blocks(
    #[case] wire: &str,
    #[case] expected: usize,
) {
    let message = messages(wire).remove(0);
    assert_eq!(count_message_marks(&message), expected);
    assert_eq!(count_message_marks(&message) > 0, expected > 0);
}

#[test]
fn a_text_system_carries_no_blocks_to_count() {
    let system = system_of(r#""be brief""#);
    assert_eq!(count_system_marks(Some(&system)), 0);
    assert!(!system_has_mark(&system));
}

#[test]
fn a_block_system_counts_its_marked_blocks() {
    let system = system_of(
        r#"[{"type": "text", "text": "a"}, {"type": "text", "text": "b", "cache_control": {}}]"#,
    );
    assert_eq!(count_system_marks(Some(&system)), 1);
    assert!(system_has_mark(&system));
}

#[test]
fn a_request_counts_messages_and_system_together() {
    let conversation = messages(
        r#"[{"role": "user", "content": "hi", "cache_control": {}}, {"role": "assistant", "content": "yo"}]"#,
    );
    let system = system_of(
        r#"[{"type": "text", "text": "a", "cache_control": {}}, {"type": "text", "text": "b"}]"#,
    );
    assert_eq!(count_request_marks(&conversation, Some(&system)), 2);
    assert_eq!(count_request_marks(&conversation, None), 1);
}
