use rstest::rstest;

use super::support;
use crate::message_utilities::validators::{
    StopSequences, ValidationError, cleanup_none_field_in_message, convert_list_message_to_dict,
    fix_openai_messages, fix_thinking_param, validate_and_fix_openai_tools,
    validate_openai_user_messages, validate_optional_params, validate_tool_choice,
};

const MEDIUM_BUDGET: u32 = 2048;

#[rstest]
#[case::missing(
    r#"{"content": "no role"}"#,
    r#"{"content": "no role", "role": "assistant"}"#
)]
#[case::empty(
    r#"{"role": "", "content": "x"}"#,
    r#"{"role": "assistant", "content": "x"}"#
)]
#[case::null(
    r#"{"role": null, "content": "x"}"#,
    r#"{"role": "assistant", "content": "x"}"#
)]
#[case::given(
    r#"{"role": "user", "content": "x"}"#,
    r#"{"role": "user", "content": "x"}"#
)]
#[tokio::test]
async fn validate_and_fix_messages_assigns_the_missing_role(
    #[case] input: &str,
    #[case] expected: &str,
) {
    let messages = support::values(&format!("[{input}]"));
    let fixed = fix_openai_messages(&messages).unwrap();
    assert_eq!(fixed, support::values(&format!("[{expected}]")));
}

#[tokio::test]
async fn validate_and_fix_messages_removes_null_fields() {
    let messages =
        support::values(r#"[{"role": "user", "content": "hi", "name": null, "function": null}]"#);
    let fixed = fix_openai_messages(&messages).unwrap();
    assert_eq!(
        fixed,
        support::values(r#"[{"role": "user", "content": "hi"}]"#)
    );
}

#[rstest]
#[case::mixed_list(
    r#"[{"role": "assistant", "tool_calls": [{"id": "c"}, "x", 5]}]"#,
    r#"[{"role": "assistant", "tool_calls": [{"id": "c"}]}]"#
)]
#[case::string(
    r#"[{"role": "assistant", "tool_calls": "x"}]"#,
    r#"[{"role": "assistant", "tool_calls": []}]"#
)]
#[case::empty_string(
    r#"[{"role": "assistant", "tool_calls": ""}]"#,
    r#"[{"role": "assistant", "tool_calls": ""}]"#
)]
#[case::object(
    r#"[{"role": "assistant", "tool_calls": {"a": 1}}]"#,
    r#"[{"role": "assistant", "tool_calls": []}]"#
)]
#[tokio::test]
async fn validate_and_fix_messages_keeps_only_dict_tool_calls(
    #[case] input: &str,
    #[case] expected: &str,
) {
    let messages = support::values(input);
    let fixed = fix_openai_messages(&messages).unwrap();
    assert_eq!(fixed, support::values(expected));
}

#[tokio::test]
async fn validate_and_fix_messages_rejects_uniterable_tool_calls() {
    let messages = support::values(r#"[{"role": "assistant", "tool_calls": 5}]"#);
    assert_eq!(
        fix_openai_messages(&messages),
        Err(ValidationError::ToolCallsNotIterable)
    );
}

#[tokio::test]
async fn validate_and_fix_messages_rejects_a_bad_user_content_type() {
    let messages = support::values(
        r#"[{"role": "assistant", "content": "a"}, {"role": "user", "content": [{"type": "text"}, {"type": "tool_use"}]}]"#,
    );
    assert_eq!(
        fix_openai_messages(&messages),
        Err(ValidationError::InvalidUserMessage { index: 1 })
    );
}

#[tokio::test]
async fn validate_and_fix_messages_rejects_a_non_object_message() {
    let messages = support::values(r#"["hi"]"#);
    let error = fix_openai_messages(&messages).unwrap_err();
    assert_eq!(error, ValidationError::NotAnObject);
    assert!(error.class().is_none());
}

#[tokio::test]
async fn convert_list_messages_rejects_a_non_object() {
    assert_eq!(
        convert_list_message_to_dict(&support::values(r#"["hi"]"#)),
        Err(ValidationError::NotAnObject)
    );
    assert_eq!(
        convert_list_message_to_dict(&support::values(r#"[{"role": "user", "content": null}]"#)),
        Ok(support::values(r#"[{"role": "user"}]"#))
    );
}

#[tokio::test]
async fn cleanup_none_keeps_every_other_value() {
    let message =
        support::value(r#"{"a": null, "b": 0, "c": false, "d": "", "e": [], "f": {}, "g": "x"}"#);
    assert_eq!(
        cleanup_none_field_in_message(&message).unwrap(),
        support::value(r#"{"b": 0, "c": false, "d": "", "e": [], "f": {}, "g": "x"}"#)
    );
}

#[tokio::test]
async fn validate_and_fix_tools_keeps_only_objects() {
    assert_eq!(validate_and_fix_openai_tools(None), None);
    let tools = support::values(r#"[{"type": "function"}, "x", 5]"#);
    assert_eq!(
        validate_and_fix_openai_tools(Some(&tools)),
        Some(support::values(r#"[{"type": "function"}]"#))
    );
}

#[rstest]
#[case::user_content_types(
    r#"[{"role": "user", "content": [{"type": "text", "text": "t"}, {"type": "image_url", "image_url": {"url": "u"}}]}]"#
)]
#[case::string_content(r#"[{"role": "user", "content": "hi"}]"#)]
#[case::null_content(r#"[{"role": "user", "content": null}]"#)]
#[case::absent_content(r#"[{"role": "user"}]"#)]
#[case::object_content(r#"[{"role": "user", "content": {"type": "text"}}]"#)]
#[case::non_dict_items(r#"[{"role": "user", "content": [5, {"type": "text"}]}]"#)]
#[case::other_roles(r#"[{"role": "assistant", "content": [{"type": "tool_use"}]}]"#)]
#[tokio::test]
async fn validate_user_messages_accepts_litellm_shapes(#[case] input: &str) {
    let messages = support::values(input);
    assert_eq!(validate_openai_user_messages(&messages), Ok(messages));
}

#[rstest]
#[case::unknown_type(r#"[{"role": "user", "content": [{"type": "tool_use"}]}]"#, 0)]
#[case::missing_type(r#"[{"role": "user", "content": [{"text": "t"}]}]"#, 0)]
#[case::second_message(
    r#"[{"role": "user", "content": "ok"}, {"role": "user", "content": [{"type": "image"}]}]"#,
    1
)]
#[tokio::test]
async fn validate_user_messages_rejects_unknown_content_types(
    #[case] input: &str,
    #[case] index: usize,
) {
    assert_eq!(
        validate_openai_user_messages(&support::values(input)),
        Err(ValidationError::InvalidUserMessage { index })
    );
}

#[tokio::test]
async fn validate_user_messages_rejects_a_message_without_a_role() {
    assert_eq!(
        validate_openai_user_messages(&support::values(r#"[{"content": "x"}]"#)),
        Err(ValidationError::InvalidMessage { index: 0 })
    );
    assert_eq!(
        validate_openai_user_messages(&support::values(r#"["x"]"#)),
        Err(ValidationError::InvalidMessage { index: 0 })
    );
}

#[rstest]
#[case::null(None)]
#[case::auto(Some("auto"))]
#[case::none(Some("none"))]
#[case::required(Some("required"))]
#[case::function(Some("function"))]
#[tokio::test]
async fn tool_choice_passes_none_and_strings_through(#[case] expected: Option<&str>) {
    let input = expected.map_or_else(|| "null".to_owned(), |text| format!("\"{text}\""));
    let choice = support::value(&input);
    assert_eq!(validate_tool_choice(&choice), Ok(choice));
}

#[rstest]
#[case::auto(r#"{"type": "auto"}"#, r#""auto""#)]
#[case::none(r#"{"type": "none"}"#, r#""none""#)]
#[case::required(r#"{"type": "required"}"#, r#""required""#)]
#[case::named(
    r#"{"type": "function", "function": {"name": "f"}}"#,
    r#"{"type": "function", "function": {"name": "f"}}"#
)]
#[case::other_with_function(
    r#"{"type": "other", "function": {"name": "f"}}"#,
    r#"{"type": "other", "function": {"name": "f"}}"#
)]
#[tokio::test]
async fn tool_choice_collapses_only_a_bare_mode(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        validate_tool_choice(&support::value(input)),
        Ok(support::value(expected))
    );
}

#[rstest]
#[case::function_only(r#"{"function": {"name": "f"}}"#)]
#[case::type_only(r#"{"type": "function"}"#)]
#[case::null_function(r#"{"type": "auto", "function": null}"#)]
#[tokio::test]
async fn tool_choice_rejects_an_object_missing_its_parts(#[case] input: &str) {
    let error = validate_tool_choice(&support::value(input)).unwrap_err();
    assert!(matches!(error, ValidationError::InvalidToolChoice { .. }));
    let class = error.class().unwrap();
    assert_eq!(class.metadata().status_code, 400);
}

#[rstest]
#[case::number("5", "number")]
#[case::boolean("true", "bool")]
#[case::list(r#"["auto"]"#, "list")]
#[tokio::test]
async fn tool_choice_rejects_other_types(#[case] input: &str, #[case] kind: &str) {
    let error = validate_tool_choice(&support::value(input)).unwrap_err();
    assert!(matches!(
        error,
        ValidationError::InvalidToolChoiceType { .. }
    ));
    assert_eq!(error.class().unwrap().metadata().status_code, 400);
    assert!(error.to_string().contains(&format!("Got={kind}.")));
    let messages = support::values(r#"[{"role": "user", "content": "hi"}]"#);
    assert_eq!(validate_openai_user_messages(&messages), Ok(messages));
}

#[tokio::test]
async fn tool_choice_passes_a_bare_mode_with_a_function_key_through() {
    let choice = support::value(r#"{"type": "auto", "function": {"name": "f"}}"#);
    assert_eq!(validate_tool_choice(&choice), Ok(choice));
}

#[rstest]
#[case::null(None, false, None)]
#[case::string(Some(StopSequences::One("x".to_owned())), false, Some(StopSequences::One("x".to_owned())))]
#[case::three(
    Some(StopSequences::Many(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])),
    false,
    Some(StopSequences::Many(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]))
)]
#[case::five(
    Some(StopSequences::Many(vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned(), "e".to_owned()])),
    false,
    Some(StopSequences::Many(vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned()]))
)]
#[case::five_unlimited(
    Some(StopSequences::Many(vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned(), "e".to_owned()])),
    true,
    Some(StopSequences::Many(vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned(), "e".to_owned()]))
)]
#[tokio::test]
async fn stop_sequences_truncate_to_four(
    #[case] stop: Option<StopSequences>,
    #[case] disabled: bool,
    #[case] expected: Option<StopSequences>,
) {
    assert_eq!(validate_optional_params(stop, disabled), expected);
}

#[rstest]
#[case::enabled(
    r#"{"type": "enabled", "budget_tokens": 7}"#,
    r#"{"type": "enabled", "budget_tokens": 7}"#
)]
#[case::camel(
    r#"{"budgetTokens": 5, "type": "enabled"}"#,
    r#"{"type": "enabled", "budget_tokens": 5}"#
)]
#[case::both(
    r#"{"budgetTokens": 5, "budget_tokens": 7}"#,
    r#"{"budget_tokens": 7}"#
)]
#[tokio::test]
async fn thinking_param_normalizes_and_keeps_dicts(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        fix_thinking_param(&support::value(input), MEDIUM_BUDGET),
        Some(support::value(expected))
    );
}

#[rstest]
#[case::true_to_medium("true", r#"{"type": "enabled", "budget_tokens": 2048}"#)]
#[case::string_passes("\"enabled\"", "\"enabled\"")]
#[tokio::test]
async fn thinking_param_maps_values(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        fix_thinking_param(&support::value(input), MEDIUM_BUDGET),
        Some(support::value(expected))
    );
}

#[rstest]
#[case::false_value("false")]
#[case::null("null")]
#[tokio::test]
async fn thinking_param_treats_false_and_null_as_absent(#[case] input: &str) {
    assert_eq!(
        fix_thinking_param(&support::value(input), MEDIUM_BUDGET),
        None
    );
}

#[tokio::test]
async fn thinking_param_moves_budget_tokens_to_the_snake_case_key() {
    assert_eq!(
        fix_thinking_param(&support::value(r#"{"budgetTokens": 5}"#), MEDIUM_BUDGET),
        Some(support::value(r#"{"budget_tokens": 5}"#))
    );
    assert_eq!(
        fix_thinking_param(
            &support::value(r#"{"budgetTokens": 5, "budget_tokens": 7}"#),
            MEDIUM_BUDGET
        ),
        Some(support::value(r#"{"budget_tokens": 7}"#))
    );
}
