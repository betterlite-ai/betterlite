use betterlite_error::error::ErrorClass;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use rstest::rstest;

use super::super::refusal::{
    ChatTurn, OutputItem, StopDetails, chat_refusal_text, refusal_stop_details,
    responses_output_refusal_text, safeguard_refusal_error, safeguard_refusal_stop_details,
};
use super::support::{json, object};

fn typed(parts: &str) -> SdkOutput {
    serde_json::from_str(&format!(
        r#"{{"type": "message", "id": "m", "role": "assistant", "status": "completed", "content": {parts}}}"#
    ))
    .expect("typed output message")
}

const TEXT_ONLY: &str = r#"[{"type": "output_text", "text": "hi", "annotations": []}]"#;

#[rstest]
#[case::with_explanation(Some("because"), Some("because"))]
#[case::without_explanation(None, None)]
fn a_refusal_stop_details_carries_only_the_explanation(
    #[case] explanation: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        refusal_stop_details(explanation.map(str::to_owned)),
        StopDetails {
            r#type: "refusal",
            category: None,
            explanation: expected.map(str::to_owned),
        }
    );
}

#[rstest]
#[case::details(
    r#"{"stop_reason": "refusal", "stop_details": {"category": "cyber"}}"#,
    Some(r#"{"category": "cyber"}"#)
)]
#[case::empty_details(r#"{"stop_reason": "refusal", "stop_details": {}}"#, Some("{}"))]
#[case::other_stop_reason(r#"{"stop_reason": "end_turn", "stop_details": {"a": 1}}"#, None)]
#[case::missing_stop_reason(r#"{"stop_details": {"a": 1}}"#, None)]
#[case::non_text_stop_reason(r#"{"stop_reason": 1, "stop_details": {"a": 1}}"#, None)]
#[case::missing_details(r#"{"stop_reason": "refusal"}"#, None)]
#[case::null_details(r#"{"stop_reason": "refusal", "stop_details": null}"#, None)]
#[case::text_details(r#"{"stop_reason": "refusal", "stop_details": "no"}"#, None)]
#[case::list_details(r#"{"stop_reason": "refusal", "stop_details": []}"#, None)]
#[case::not_an_object(r#"["refusal"]"#, None)]
fn safeguard_details_need_a_refusal_stop_reason_and_an_object(
    #[case] sent: &str,
    #[case] details: Option<&str>,
) {
    let response = json(sent);
    assert_eq!(
        safeguard_refusal_stop_details(&response),
        details.map(object).as_ref()
    );
}

#[rstest]
#[case::text_category(
    r#"{"category": "cyber"}"#,
    "Anthropic safeguard refusal (category: cyber)."
)]
#[case::missing_category("{}", "Anthropic safeguard refusal (category: None).")]
#[case::null_category(
    r#"{"category": null}"#,
    "Anthropic safeguard refusal (category: None)."
)]
fn safeguard_refusal_is_a_content_policy_violation(#[case] details: &str, #[case] message: &str) {
    let class = safeguard_refusal_error("claude-opus-4-8", &object(details));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.model.as_deref(), Some("claude-opus-4-8"));
    assert_eq!(metadata.provider.as_deref(), Some("anthropic"));
    let ErrorClass::ContentPolicyViolation(violation) = class else {
        panic!("expected a content policy violation");
    };
    assert_eq!(violation.message, message);
}

#[rstest]
#[case::refusal_field(r#""no""#, "null", Some("no"))]
#[case::refusal_wins(r#""no""#, r#"{"refusal": "other"}"#, Some("no"))]
#[case::empty_refusal_falls_back(r#""""#, r#"{"refusal": "other"}"#, Some("other"))]
#[case::null_refusal_falls_back("null", r#"{"refusal": "other"}"#, Some("other"))]
#[case::non_text_refusal_falls_back("3", r#"{"refusal": "other"}"#, Some("other"))]
#[case::empty_fallback("null", r#"{"refusal": ""}"#, None)]
#[case::non_text_fallback("null", r#"{"refusal": 4}"#, None)]
#[case::missing_fallback("null", r#"{"other": "x"}"#, None)]
#[case::fields_not_an_object("null", r#"["no"]"#, None)]
#[case::fields_null("null", "null", None)]
fn chat_refusal_prefers_the_refusal_field(
    #[case] sent_refusal: &str,
    #[case] sent_fields: &str,
    #[case] expected: Option<&str>,
) {
    let refusal = json(sent_refusal);
    let fields = json(sent_fields);
    let turn = ChatTurn {
        refusal: &refusal,
        provider_specific_fields: &fields,
    };
    assert_eq!(chat_refusal_text(&turn), expected);
}

#[rstest]
#[case::refusal_part(&[r#"[{"type": "refusal", "refusal": "no"}]"#], Some("no"))]
#[case::text_then_refusal(
    &[r#"[{"type": "output_text", "text": "hi", "annotations": []}, {"type": "refusal", "refusal": "later"}]"#],
    Some("later")
)]
#[case::text_only(&[TEXT_ONLY], None)]
#[case::empty_refusal_then_next(
    &[r#"[{"type": "refusal", "refusal": ""}, {"type": "refusal", "refusal": "next"}]"#],
    Some("next")
)]
#[case::empty_refusal_only(&[r#"[{"type": "refusal", "refusal": ""}]"#], None)]
#[case::first_message_wins(
    &[r#"[{"type": "refusal", "refusal": "one"}]"#, r#"[{"type": "refusal", "refusal": "two"}]"#],
    Some("one")
)]
#[case::skips_a_message_without_refusal(
    &[TEXT_ONLY, r#"[{"type": "refusal", "refusal": "two"}]"#],
    Some("two")
)]
fn typed_output_refusals_come_from_message_parts(
    #[case] messages: &[&str],
    #[case] expected: Option<&str>,
) {
    let outputs: Vec<SdkOutput> = messages.iter().map(|parts| typed(parts)).collect();
    let items: Vec<OutputItem<'_>> = outputs.iter().map(OutputItem::Typed).collect();
    assert_eq!(responses_output_refusal_text(&items), expected);
}

#[test]
fn a_typed_item_that_is_not_a_message_has_no_refusal() {
    let reasoning: SdkOutput =
        serde_json::from_str(r#"{"type": "reasoning", "id": "r", "summary": []}"#)
            .expect("reasoning item");
    assert_eq!(
        responses_output_refusal_text(&[OutputItem::Typed(&reasoning)]),
        None
    );
}

#[rstest]
#[case::refusal_part(
    r#"{"type": "message", "content": [{"type": "refusal", "refusal": "no"}]}"#,
    Some("no")
)]
#[case::text_part_only(
    r#"{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}"#,
    None
)]
#[case::non_text_refusal(
    r#"{"type": "message", "content": [{"type": "refusal", "refusal": 3}]}"#,
    None
)]
#[case::empty_refusal_then_next(
    r#"{"type": "message", "content": [{"type": "refusal", "refusal": ""}, {"type": "refusal", "refusal": "next"}]}"#,
    Some("next")
)]
#[case::other_item_type(
    r#"{"type": "reasoning", "content": [{"type": "refusal", "refusal": "no"}]}"#,
    None
)]
#[case::missing_item_type(r#"{"content": [{"type": "refusal", "refusal": "no"}]}"#, None)]
#[case::missing_content(r#"{"type": "message"}"#, None)]
#[case::content_not_a_list(r#"{"type": "message", "content": "no"}"#, None)]
#[case::part_not_an_object(r#"{"type": "message", "content": ["refusal"]}"#, None)]
#[case::item_not_an_object(r#""message""#, None)]
fn raw_output_dicts_count_when_they_are_messages(
    #[case] sent: &str,
    #[case] expected: Option<&str>,
) {
    let item = json(sent);
    assert_eq!(
        responses_output_refusal_text(&[OutputItem::Raw(&item)]),
        expected
    );
}

#[test]
fn an_empty_output_has_no_refusal() {
    assert_eq!(responses_output_refusal_text(&[]), None);
}
