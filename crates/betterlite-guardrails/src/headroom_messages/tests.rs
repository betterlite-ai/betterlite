#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::{
    HeadroomMessagesError, flatten_for_compression, protected_indices, restore_content_shapes,
    restore_protected_messages, retrieval_result_indices, retrieve_call_ids,
};
use crate::primitives::json_value::JsonValue;

fn rows(source: &str) -> Vec<JsonValue> {
    serde_json::from_str(source).unwrap()
}

fn row(source: &str) -> JsonValue {
    serde_json::from_str(source).unwrap()
}

fn indices(values: &[usize]) -> BTreeSet<usize> {
    values.iter().copied().collect()
}

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn protected_indices_take_the_system_last_user_and_last_assistant_rows() {
    let messages = rows(
        r#"[
            {"role": "system", "content": "s"},
            {"role": "user", "content": "first"},
            {"role": "assistant", "content": "a"},
            {"role": "user", "content": "last"}
        ]"#,
    );
    assert_eq!(
        protected_indices(&messages, &BTreeSet::new()),
        indices(&[0, 2, 3])
    );
}

#[test]
fn protected_indices_add_every_assistant_row() {
    let messages = rows(
        r#"[
            {"role": "user", "content": "u1"},
            {"role": "assistant", "content": "a1"},
            {"role": "user", "content": "u2"},
            {"role": "assistant", "content": "a2"},
            {"role": "user", "content": "u3"}
        ]"#,
    );
    assert_eq!(
        protected_indices(&messages, &BTreeSet::new()),
        indices(&[1, 3, 4])
    );
}

#[test]
fn protected_indices_expand_a_protected_member_over_its_tool_exchange() {
    let messages = rows(
        r#"[
            {"role": "user", "content": "u"},
            {"role": "assistant", "content": null, "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call-1", "content": "result"},
            {"role": "user", "content": "next"}
        ]"#,
    );
    assert_eq!(
        protected_indices(&messages, &BTreeSet::new()),
        indices(&[1, 2, 3])
    );
}

#[test]
fn protected_indices_leave_tool_results_of_a_later_assistant_compressible() {
    let messages = rows(
        r#"[
            {"role": "user", "content": "u"},
            {"role": "assistant", "content": null, "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call-1", "content": "result"},
            {"role": "assistant", "content": "a"},
            {"role": "user", "content": "next"}
        ]"#,
    );
    assert_eq!(
        protected_indices(&messages, &BTreeSet::new()),
        indices(&[1, 3, 4])
    );
}

#[test]
fn protected_indices_add_retrieval_result_rows() {
    let messages = rows(
        r#"[
            {"role": "user", "content": "u"},
            {"role": "assistant", "content": null, "tool_calls": [{"id": "call-9", "type": "function", "function": {"name": "headroom_retrieve", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call-9", "content": "recovered"},
            {"role": "assistant", "content": "a"},
            {"role": "user", "content": "next"}
        ]"#,
    );
    assert_eq!(
        protected_indices(&messages, &BTreeSet::new()),
        indices(&[1, 2, 3, 4])
    );
}

#[test]
fn protected_indices_use_extra_retrieve_ids_for_anthropic_results() {
    let messages = rows(
        r#"[
            {"role": "user", "content": "u"},
            {"role": "tool", "tool_call_id": "toolu-1", "content": "recovered"},
            {"role": "user", "content": "next"}
        ]"#,
    );
    let protected = protected_indices(&messages, &ids(&["toolu-1"]));
    assert!(protected.contains(&1));
    assert_eq!(
        protected_indices(&messages, &BTreeSet::new()),
        indices(&[2])
    );
}

#[test]
fn protected_retrieve_call_ids_read_openai_assistant_tool_calls() {
    let messages = row(r#"[
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call-1", "type": "function", "function": {"name": "headroom_retrieve", "arguments": "{}"}},
                {"id": "call-2", "type": "function", "function": {"name": "mcp__headroom__headroom_retrieve", "arguments": "{}"}},
                {"id": "call-3", "type": "function", "function": {"name": "lookup", "arguments": "{}"}},
                {"id": "", "type": "function", "function": {"name": "headroom_retrieve", "arguments": "{}"}}
            ]}
        ]"#);
    assert_eq!(
        retrieve_call_ids(Some(&messages)),
        ids(&["call-1", "call-2"])
    );
}

#[test]
fn protected_retrieve_call_ids_read_anthropic_tool_use_blocks() {
    let messages = row(r#"[
            {"role": "user", "content": [
                {"type": "tool_use", "id": "toolu-1", "name": "headroom_retrieve", "input": {}},
                {"type": "tool_use", "id": "", "name": "mcp__server__headroom_retrieve", "input": {}},
                {"type": "tool_use", "id": "toolu-3", "name": "lookup", "input": {}},
                {"type": "text", "text": "hi"},
                {"type": "text", "id": "text-1", "name": "headroom_retrieve", "text": "not a call"}
            ]}
        ]"#);
    assert_eq!(retrieve_call_ids(Some(&messages)), ids(&["", "toolu-1"]));
}

#[test]
fn protected_retrieve_call_ids_ignore_non_assistant_openai_rows() {
    let messages = row(r#"[
            {"role": "user", "content": null, "tool_calls": [
                {"id": "call-1", "type": "function", "function": {"name": "headroom_retrieve", "arguments": "{}"}}
            ]}
        ]"#);
    assert_eq!(retrieve_call_ids(Some(&messages)), BTreeSet::new());
}

#[rstest]
#[case::missing(None)]
#[case::not_a_list(Some(r#"{"role": "assistant"}"#))]
fn protected_retrieve_call_ids_are_empty_without_a_message_list(#[case] messages: Option<&str>) {
    let message = messages.map(row);
    assert_eq!(retrieve_call_ids(message.as_ref()), BTreeSet::new());
}

#[test]
fn protected_retrieval_result_rows_need_a_matching_id() {
    let messages = rows(
        r#"[
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call-1", "type": "function", "function": {"name": "headroom_retrieve", "arguments": "{}"}}
            ]},
            {"role": "tool", "tool_call_id": "call-1", "content": "recovered"},
            {"role": "function", "tool_call_id": "call-2", "content": "other"},
            {"role": "user", "tool_call_id": "call-1", "content": "not a result"},
            {"role": "function", "tool_call_id": "call-1", "content": "recovered again"}
        ]"#,
    );
    assert_eq!(
        retrieval_result_indices(&messages, &BTreeSet::new()),
        indices(&[1, 4])
    );
}

#[test]
fn protected_retrieval_result_rows_are_empty_without_ids() {
    let messages = rows(
        r#"[
            {"role": "tool", "tool_call_id": "call-1", "content": "result"}
        ]"#,
    );
    assert_eq!(
        retrieval_result_indices(&messages, &BTreeSet::new()),
        BTreeSet::new()
    );
}

#[test]
fn protected_retrieval_result_rows_ignore_anthropic_blocks_in_the_messages() {
    let messages = rows(
        r#"[
            {"role": "user", "content": [
                {"type": "tool_use", "id": "toolu-1", "name": "headroom_retrieve", "input": {}}
            ]},
            {"role": "tool", "tool_call_id": "toolu-1", "content": "recovered"}
        ]"#,
    );
    assert_eq!(
        retrieval_result_indices(&messages, &BTreeSet::new()),
        BTreeSet::new()
    );
}

#[test]
fn flatten_all_text_parts_replaces_the_list_with_its_joined_text() {
    let flattened = flatten_for_compression(&rows(
        r#"[
            {"role": "user", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]},
            {"role": "assistant", "content": "plain"}
        ]"#,
    ));
    assert_eq!(
        flattened,
        rows(
            r#"[
                {"role": "user", "content": "a\n\nb"},
                {"role": "assistant", "content": "plain"}
            ]"#
        )
    );
}

#[test]
fn flatten_keeps_rows_with_a_non_text_part() {
    let source = r#"[
        {"role": "user", "content": [
            {"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}},
            {"type": "image_url", "image_url": {"url": "u"}},
            {"type": "text", "text": "b"}
        ]}
    ]"#;
    assert_eq!(flatten_for_compression(&rows(source)), rows(source));
}

#[rstest]
#[case::empty_parts(r#"[{"role": "user", "content": []}]"#)]
#[case::empty_text(r#"[{"role": "user", "content": [{"type": "text", "text": ""}]}]"#)]
#[case::missing_content(r#"[{"role": "user"}]"#)]
#[case::null_content(r#"[{"role": "user", "content": null}]"#)]
fn flatten_keeps_rows_whose_text_is_empty(#[case] source: &str) {
    assert_eq!(flatten_for_compression(&rows(source)), rows(source));
}

#[test]
fn restore_shapes_keep_the_original_parts_for_an_untouched_row() {
    let originals = rows(
        r#"[
            {"role": "user", "content": [
                {"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "b", "cache_control": {"type": "ephemeral", "ttl": "5m"}}
            ]}
        ]"#,
    );
    let returned = rows(r#"[{"role": "user", "content": "a\n\nb"}]"#);
    assert_eq!(restore_content_shapes(&originals, &returned), originals);
}

#[test]
fn restore_shapes_merge_rewritten_text_with_the_last_cache_control() {
    let originals = rows(
        r#"[
            {"role": "user", "content": [
                {"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "b", "cache_control": {"type": "ephemeral", "ttl": "5m"}}
            ]}
        ]"#,
    );
    let returned = rows(r#"[{"role": "user", "content": "shortened"}]"#);
    assert_eq!(
        restore_content_shapes(&originals, &returned),
        rows(
            r#"[{"role": "user", "content": [
                {"type": "text", "text": "shortened", "cache_control": {"type": "ephemeral", "ttl": "5m"}}
            ]}]"#
        )
    );
}

#[test]
fn restore_shapes_adopt_the_returned_rows_when_the_count_differs() {
    let originals = rows(r#"[{"role": "user", "content": "a"}]"#);
    let returned = rows(r#"[{"role": "user", "content": "b"}, {"role": "user", "content": "c"}]"#);
    assert_eq!(restore_content_shapes(&originals, &returned), returned);
}

#[test]
fn restore_shapes_adopt_the_returned_rows_when_a_role_differs() {
    let originals = rows(r#"[{"role": "user", "content": [{"type": "text", "text": "a"}]}]"#);
    let returned = rows(r#"[{"role": "assistant", "content": "b"}]"#);
    assert_eq!(restore_content_shapes(&originals, &returned), returned);
}

#[test]
fn restore_protected_puts_the_held_rows_back_at_their_positions() {
    let messages = rows(
        r#"[
            {"role": "system", "content": "s"},
            {"role": "user", "content": "u1"},
            {"role": "assistant", "content": "a"},
            {"role": "user", "content": "u2"}
        ]"#,
    );
    let compressed = rows(
        r#"[
            {"role": "tool", "tool_call_id": "x", "content": "sent one"}
        ]"#,
    );
    let restored =
        restore_protected_messages(&messages, &compressed, &indices(&[0, 2, 3])).unwrap();
    assert_eq!(
        restored,
        rows(
            r#"[{"role": "system", "content": "s"}, {"role": "tool", "tool_call_id": "x", "content": "sent one"}, {"role": "assistant", "content": "a"}, {"role": "user", "content": "u2"}]"#
        )
    );
}

#[test]
fn restore_protected_ignores_rows_beyond_the_sent_count() {
    let messages =
        rows(r#"[{"role": "user", "content": "u"}, {"role": "assistant", "content": "a"}]"#);
    let compressed = rows(
        r#"[
            {"role": "user", "content": "compressed"},
            {"role": "assistant", "content": "extra"}
        ]"#,
    );
    let restored = restore_protected_messages(&messages, &compressed, &indices(&[1])).unwrap();
    assert_eq!(
        restored,
        rows(
            r#"[{"role": "user", "content": "compressed"}, {"role": "assistant", "content": "a"}]"#
        )
    );
}

#[test]
fn restore_protected_rejects_a_short_compressed_list() {
    let messages =
        rows(r#"[{"role": "user", "content": "u"}, {"role": "assistant", "content": "a"}]"#);
    let error = restore_protected_messages(&messages, &[], &BTreeSet::new()).unwrap_err();
    assert_eq!(
        error,
        HeadroomMessagesError::ShortCompressed {
            sent: 2,
            returned: 0
        }
    );
}
