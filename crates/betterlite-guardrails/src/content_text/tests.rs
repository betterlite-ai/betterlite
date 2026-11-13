#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{content_to_text, is_all_text_parts, merge_rewritten_text_parts};

fn json(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

fn parts(source: &str) -> Vec<GuardrailValue> {
    let GuardrailValue::Array(items) = json(source) else {
        panic!("not an array: {source}");
    };
    items
}

#[rstest]
#[case::plain_string(r#""hello""#, "hello")]
#[case::empty_string(r#""""#, "")]
#[case::text_parts_joined(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#, "a\n\nb")]
#[case::non_text_parts_skipped(r#"[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"u"}},{"type":"text","text":"b"}]"#, "a\n\nb")]
#[case::empty_text_part_still_joins(
    r#"[{"type":"text","text":""},{"type":"text","text":"b"}]"#,
    "\n\nb"
)]
#[case::part_without_text_skipped(r#"[{"type":"text"},{"type":"text","text":"b"}]"#, "b")]
#[case::part_with_null_text_skipped(
    r#"[{"type":"text","text":null},{"type":"text","text":"b"}]"#,
    "b"
)]
#[case::part_without_type_skipped(r#"[{"text":"a"},{"type":"text","text":"b"}]"#, "b")]
#[case::string_parts_skipped(r#"["a",{"type":"text","text":"b"}]"#, "b")]
#[case::empty_array("[]", "")]
#[case::null("null", "")]
#[case::number("5", "")]
#[case::object(r#"{"type":"text","text":"a"}"#, "")]
fn content_becomes_text(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(content_to_text(&json(source)), expected);
}

#[rstest]
#[case::one_text_part(r#"[{"type":"text","text":"a"}]"#, true)]
#[case::two_text_parts(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#, true)]
#[case::text_part_without_text_field(r#"[{"type":"text"}]"#, true)]
#[case::mixed_with_image(r#"[{"type":"text","text":"a"},{"type":"image_url"}]"#, false)]
#[case::part_without_type(r#"[{"text":"a"}]"#, false)]
#[case::part_with_null_type(r#"[{"type":null,"text":"a"}]"#, false)]
#[case::string_part(r#"["a"]"#, false)]
#[case::empty_array("[]", false)]
#[case::string_content(r#""a""#, false)]
#[case::null_content("null", false)]
fn only_all_text_part_lists_qualify(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(is_all_text_parts(&json(source)), expected);
}

#[rstest]
#[case::single_part(r#"[{"type":"text","text":"a"}]"#, r#"[{"type":"text","text":"new"}]"#)]
#[case::many_parts_collapse_into_one(
    r#"[{"type":"text","text":"a"},{"type":"text","text":"b"},{"type":"text","text":"c"}]"#,
    r#"[{"type":"text","text":"new"}]"#
)]
#[case::first_part_fields_are_the_base(
    r#"[{"type":"text","text":"a","citations":[1]},{"type":"text","text":"b","extra":2}]"#,
    r#"[{"type":"text","text":"new","citations":[1]}]"#
)]
#[case::the_last_breakpoint_is_kept(
    r#"[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"5m"}},{"type":"text","text":"b"},{"type":"text","text":"c","cache_control":{"type":"ephemeral","ttl":"1h"}}]"#,
    r#"[{"type":"text","text":"new","cache_control":{"type":"ephemeral","ttl":"1h"}}]"#
)]
#[case::a_null_breakpoint_does_not_replace_an_earlier_one(
    r#"[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b","cache_control":null}]"#,
    r#"[{"type":"text","text":"new","cache_control":{"type":"ephemeral"}}]"#
)]
#[case::a_breakpoint_on_a_later_part_moves_to_the_merged_part(
    r#"[{"type":"text","text":"a"},{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}]"#,
    r#"[{"type":"text","text":"new","cache_control":{"type":"ephemeral"}}]"#
)]
#[case::non_object_parts_are_ignored(
    r#"["x",{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]"#,
    r#"[{"type":"text","text":"new","cache_control":{"type":"ephemeral"}}]"#
)]
#[case::no_object_part_builds_a_text_part("[]", r#"[{"type":"text","text":"new"}]"#)]
fn rewritten_text_parts_merge_into_one_part(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        merge_rewritten_text_parts(&parts(source), "new"),
        parts(expected)
    );
}
