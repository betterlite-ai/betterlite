#![cfg(test)]

use rstest::rstest;

use super::response;
use crate::straiker::v3::fixtures::{as_json, object, parsed};

#[rstest]
#[case::no_verdict(r"{}", "null")]
#[case::ask_is_undecided(r#"{"action":"ask"}"#, "null")]
#[case::hook_ask_is_undecided(r#"{"hookSpecificOutput":{"permissionDecision":"ask"}}"#, "null")]
#[case::allow_reads_as_no_action(
    r#"{"action":"allow"}"#,
    r#"{"action":"NONE","blocked_reason":null,"blocked_by":[],"turnId":null}"#
)]
#[case::detect_mode_controls_do_not_block(
    r#"{"action":"NONE","controls":["pii"]}"#,
    r#"{"action":"NONE","blocked_reason":null,"blocked_by":[],"turnId":null}"#
)]
#[case::block_without_reason_names_policy(r#"{"action":"BLOCKED"}"#, r#"{"action":"BLOCKED","blocked_reason":"Straiker blocked this turn: policy","blocked_by":[],"turnId":null}"#)]
#[case::deny_decision_blocks(
    r#"{"hookSpecificOutput":{"permissionDecision":"deny"},"straiker":{"deny_reason":" no pii "}}"#,
    r#"{"action":"BLOCKED","blocked_reason":"no pii","blocked_by":[],"turnId":null}"#
)]
#[case::blocked_by_blocks_without_decision(r#"{"blocked_by":["pii","exfil"]}"#, r#"{"action":"BLOCKED","blocked_reason":"Straiker blocked this turn: exfil, pii","blocked_by":["exfil","pii"],"turnId":null}"#)]
#[case::blocked_by_blocks_an_ask(r#"{"action":"ask","blocked_by":["pii"]}"#, r#"{"action":"BLOCKED","blocked_reason":"Straiker blocked this turn: pii","blocked_by":["pii"],"turnId":null}"#)]
#[case::blocked_by_beats_an_allow(r#"{"action":"allow","blocked_by":["pii"]}"#, r#"{"action":"BLOCKED","blocked_reason":"Straiker blocked this turn: pii","blocked_by":["pii"],"turnId":null}"#)]
#[case::block_message_comes_first(
    r#"{"action":"block","block_message":"stop","deny_reason":"why","stopReason":"later"}"#,
    r#"{"action":"BLOCKED","blocked_reason":"stop","blocked_by":[],"turnId":null}"#
)]
#[case::blank_block_message_falls_to_deny_reason(
    r#"{"action":"block","block_message":"  ","deny_reason":"why"}"#,
    r#"{"action":"BLOCKED","blocked_reason":"why","blocked_by":[],"turnId":null}"#
)]
#[case::stop_reason_is_read_from_the_body(
    r#"{"straiker":{"action":"deny"},"stopReason":"halted"}"#,
    r#"{"action":"BLOCKED","blocked_reason":"halted","blocked_by":[],"turnId":null}"#
)]
#[case::allow_has_no_reason_even_with_message(
    r#"{"action":"allow","block_message":"stop"}"#,
    r#"{"action":"NONE","blocked_reason":null,"blocked_by":[],"turnId":null}"#
)]
#[case::turn_id_from_verdict(
    r#"{"straiker":{"action":"allow","turn_id":"t1"},"turn_id":"t2"}"#,
    r#"{"action":"NONE","blocked_reason":null,"blocked_by":[],"turnId":"t1"}"#
)]
#[case::turn_id_from_body(
    r#"{"straiker":{"action":"allow","turn_id":""},"turn_id":"t2"}"#,
    r#"{"action":"NONE","blocked_reason":null,"blocked_by":[],"turnId":"t2"}"#
)]
fn maps_verdict(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(as_json(&response(&object(source))), parsed(expected));
}
