#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::{AFTER_SCOPING, NOTHING_SCANNABLE, not_run_reason};
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, values};

#[rstest]
#[case::string_content(json(r#"[{"role": "user", "content": "hi"}]"#), Some(AFTER_SCOPING))]
#[case::empty_string_still_counts(
    json(r#"[{"role": "user", "content": ""}]"#),
    Some(AFTER_SCOPING)
)]
#[case::list_text(
    json(r#"[{"content": [{"type": "text", "text": "hi"}]}]"#),
    Some(AFTER_SCOPING)
)]
#[case::tool_calls_only(
    json(r#"[{"role": "assistant", "tool_calls": [{"id": "1"}]}]"#),
    Some(AFTER_SCOPING)
)]
#[case::text_beside_an_image(
    json(r#"[{"content": [{"type": "image_url", "image_url": {"url": "u"}}, {"type": "text", "text": "hi"}]}]"#),
    Some(AFTER_SCOPING),
)]
#[case::image_only_is_never_scanned(
    json(r#"[{"content": [{"type": "image_url", "image_url": {"url": "u"}}]}]"#),
    None
)]
#[case::image_in_a_later_message(
    json(r#"[{"role": "user"}, {"content": [{"type": "image_url", "image_url": "u"}]}]"#),
    None
)]
#[case::no_messages(json("[]"), Some(NOTHING_SCANNABLE))]
#[case::empty_message(json(r#"[{"role": "user"}]"#), Some(NOTHING_SCANNABLE))]
#[case::null_content(
    json(r#"[{"role": "user", "content": null}]"#),
    Some(NOTHING_SCANNABLE)
)]
#[case::image_without_a_url(
    json(r#"[{"content": [{"type": "image_url", "image_url": {}}]}]"#),
    Some(NOTHING_SCANNABLE)
)]
#[case::empty_tool_call_list(json(r#"[{"tool_calls": []}]"#), Some(NOTHING_SCANNABLE))]
fn why_a_scan_did_not_run(#[case] messages: JsonValue, #[case] expected: Option<&str>) {
    assert_eq!(not_run_reason(&values(messages)), expected);
}
