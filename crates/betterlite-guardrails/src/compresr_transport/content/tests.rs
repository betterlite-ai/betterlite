#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::replace_text_in_content;

fn value(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

#[test]
fn string_content_takes_the_compressed_text() {
    assert_eq!(
        replace_text_in_content(&value(r#""original""#), "compressed"),
        value(r#""compressed""#)
    );
}

#[test]
fn all_text_parts_merge_into_one_part() {
    let content = value(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#);
    assert_eq!(
        replace_text_in_content(&content, "compressed"),
        value(r#"[{"type":"text","text":"compressed"}]"#)
    );
}

#[test]
fn the_merged_part_keeps_the_last_cache_control_breakpoint() {
    let content = value(
        r#"[
            {"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"5m"}},
            {"type":"text","text":"b","cache_control":{"type":"ephemeral","ttl":"1h"}}
        ]"#,
    );
    assert_eq!(
        replace_text_in_content(&content, "compressed"),
        value(
            r#"[{"type":"text","text":"compressed","cache_control":{"type":"ephemeral","ttl":"1h"}}]"#
        )
    );
}

#[test]
fn mixed_part_lists_are_left_alone() {
    let content = value(
        r#"[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"https://x/y.png"}}]"#,
    );
    assert_eq!(replace_text_in_content(&content, "compressed"), content);
}

#[test]
fn null_content_is_left_alone() {
    assert_eq!(
        replace_text_in_content(&GuardrailValue::Null, "compressed"),
        GuardrailValue::Null
    );
}
