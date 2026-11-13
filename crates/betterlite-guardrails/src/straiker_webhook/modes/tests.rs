#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{ModeError, configured_modes};

fn hook(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::plain_hook(Some(r#""pre_call""#), Ok(Some(vec!["pre_call".to_owned()])))]
#[case::absent_hook(None, Ok(None))]
#[case::null_hook(Some("null"), Ok(None))]
#[case::number_hook(Some("7"), Ok(None))]
#[case::list_of_hooks(
    Some(r#"["pre_call","post_call"]"#),
    Ok(Some(vec!["pre_call".to_owned(), "post_call".to_owned()]))
)]
#[case::list_dedupes_in_order(
    Some(r#"["post_call","pre_call","post_call"]"#),
    Ok(Some(vec!["post_call".to_owned(), "pre_call".to_owned()]))
)]
#[case::empty_list(Some("[]"), Ok(None))]
#[case::mode_default_before_tags(
    Some(r#"{"tags":{"chat":["post_call","pre_call"],"mcp":"during_mcp_call"},"default":"pre_call"}"#),
    Ok(Some(vec![
        "pre_call".to_owned(),
        "post_call".to_owned(),
        "during_mcp_call".to_owned(),
    ]))
)]
#[case::mode_without_default(
    Some(r#"{"tags":{"chat":"post_call"}}"#),
    Ok(Some(vec!["post_call".to_owned()]))
)]
#[case::mode_null_default(Some(r#"{"tags":{},"default":null}"#), Ok(None))]
#[case::mode_null_tag(Some(r#"{"tags":{"chat":null}}"#), Ok(None))]
#[case::mode_dedupes_default_and_tags(
    Some(r#"{"tags":{"chat":["pre_call"]},"default":"pre_call"}"#),
    Ok(Some(vec!["pre_call".to_owned()]))
)]
#[case::mode_ignores_unknown_keys(
    Some(r#"{"tags":{"chat":"post_call"},"extra":true}"#),
    Ok(Some(vec!["post_call".to_owned()]))
)]
#[case::mode_missing_tags(Some("{}"), Err(ModeError::MissingTags))]
#[case::tags_not_an_object(
    Some(r#"{"tags":"pre_call"}"#),
    Err(ModeError::NotAMode {
        field: "tags",
        kind: "a string",
    })
)]
#[case::list_entry_not_a_string(
    Some(r#"["pre_call",7]"#),
    Err(ModeError::NotAMode {
        field: "entries",
        kind: "a number",
    })
)]
#[case::tag_value_not_a_mode(
    Some(r#"{"tags":{"chat":true}}"#),
    Err(ModeError::NotAMode {
        field: "tags",
        kind: "a boolean",
    })
)]
#[case::default_not_a_mode(
    Some(r#"{"tags":{},"default":{"x":1}}"#),
    Err(ModeError::NotAMode {
        field: "default",
        kind: "an object",
    })
)]
fn straiker_webhook_context_tests_behavior(
    #[case] source: Option<&str>,
    #[case] expected: Result<Option<Vec<String>>, ModeError>,
) {
    let event_hook = source.map(hook);
    assert_eq!(configured_modes(event_hook.as_ref()), expected);
}
