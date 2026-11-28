use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::api::messages_compaction::error::SpecError;
use crate::api::messages_compaction::normalize::normalize_context_edit_shapes as normalize_shapes;

use super::support::{decided, parse};

// the DoD shapes: a missing, null or edit-less spec and a non-list edits value leave nothing to dispatch
#[rstest]
#[case::absent(None, "[]")]
#[case::null(Some("null"), "[]")]
#[case::empty_object(Some("{}"), "[]")]
#[case::null_edits(Some(r#"{"edits":null}"#), "[]")]
#[case::empty_edits(Some(r#"{"edits":[]}"#), "[]")]
#[case::text_edits(Some(r#"{"edits":"bad"}"#), "[]")]
#[case::object_edits(Some(r#"{"edits":{"type":"clear_tool_uses_20250919"}}"#), "[]")]
#[case::keeps_dict_edits(
    Some(r#"{"edits":[7,{},{"type":"clear_tool_uses_20250919"}]}"#),
    r#"[{},{"type":"clear_tool_uses_20250919"}]"#
)]
#[case::maps_compaction_threshold(
    Some(r#"[{"type":"compaction","compact_threshold":60000.9}]"#),
    r#"[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":60000}}]"#
)]
#[case::keeps_an_explicit_trigger(
    Some(
        r#"[{"type":"compaction","compact_threshold":60000.9,
            "trigger":{"type":"output_tokens","value":70000}}]"#
    ),
    r#"[{"type":"compact_20260112","trigger":{"type":"output_tokens","value":70000}}]"#
)]
#[case::drops_unknown_openai_entries(
    Some(
        r#"[{"type":"future_edit","compact_threshold":60000},
            {"type":"compaction","compact_threshold":70000}]"#
    ),
    r#"[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":70000}}]"#
)]
#[case::discards_an_all_unknown_openai_list(
    Some(r#"[{"type":"future_edit","compact_threshold":60000},7]"#),
    "[]"
)]
#[case::passes_native_unknown_edits_through(
    Some(r#"{"edits":[{"type":"future_edit","value":1}]}"#),
    r#"[{"type":"future_edit","value":1}]"#
)]
fn normalize_context_edit_shapes(#[case] raw: Option<&str>, #[case] expected: &str) {
    let spec = raw.map(parse);
    let edits: Vec<Json> = normalize_shapes(spec.as_ref())
        .expect("the spec normalizes")
        .into_iter()
        .map(Json::Object)
        .collect();
    assert_eq!(edits, decided(expected));
}

#[test]
fn normalize_context_edit_shapes_surfaces_a_bad_threshold() {
    let spec = parse(r#"[{"type":"compaction","compact_threshold":1e30}]"#);
    let error: SpecError =
        normalize_shapes(Some(&spec)).expect_err("a threshold outside i64 has no token count");
    assert_eq!(
        error.to_string(),
        "the compact threshold is not a whole number of tokens"
    );
}
