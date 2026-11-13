#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;
use rstest::rstest;

use super::error::SelectionError;
use super::{CallbackNames, can_modify_guardrails, guardrail_applies};

fn object(source: &str) -> Object {
    let GuardrailValue::Object(fields) = serde_json::from_str(source).unwrap() else {
        panic!("not an object: {source}");
    };
    fields
}

fn callback_names() -> CallbackNames {
    CallbackNames::from([
        (
            "prompt_injection".to_owned(),
            vec!["lakera_prompt_injection".to_owned()],
        ),
        (
            "pii".to_owned(),
            vec!["presidio".to_owned(), "pii_masking".to_owned()],
        ),
        ("nothing".to_owned(), Vec::new()),
    ])
}

#[rstest]
#[case::no_team(None, true)]
#[case::no_guardrails_key(Some(r#"{"other":1}"#), true)]
#[case::empty_metadata(Some("{}"), true)]
#[case::locked(Some(r#"{"guardrails":{"modify_guardrails":false}}"#), false)]
#[case::explicitly_allowed(Some(r#"{"guardrails":{"modify_guardrails":true}}"#), true)]
#[case::flag_unset(Some(r#"{"guardrails":{}}"#), true)]
#[case::flag_null(Some(r#"{"guardrails":{"modify_guardrails":null}}"#), true)]
#[case::flag_zero_is_not_false(Some(r#"{"guardrails":{"modify_guardrails":0}}"#), true)]
#[case::flag_text_is_not_false(Some(r#"{"guardrails":{"modify_guardrails":"false"}}"#), true)]
#[case::guardrails_not_an_object(Some(r#"{"guardrails":false}"#), true)]
#[case::guardrails_list(Some(r#"{"guardrails":["modify_guardrails"]}"#), true)]
#[case::guardrails_null(Some(r#"{"guardrails":null}"#), true)]
fn a_team_locks_guardrails_with_an_explicit_false(
    #[case] metadata: Option<&str>,
    #[case] expected: bool,
) {
    let parsed = metadata.map(object);
    assert_eq!(can_modify_guardrails(parsed.as_ref()), expected);
}

#[rstest]
#[case::requested_and_enabled(
    r#"{"metadata":{"guardrails":{"prompt_injection":true}}}"#,
    "lakera_prompt_injection",
    true
)]
#[case::other_callback_is_not_requested(
    r#"{"metadata":{"guardrails":{"prompt_injection":true}}}"#,
    "presidio",
    false
)]
#[case::set_to_false_skips_its_callbacks(
    r#"{"metadata":{"guardrails":{"prompt_injection":false}}}"#,
    "lakera_prompt_injection",
    false
)]
#[case::false_for_one_leaves_the_other(
    r#"{"metadata":{"guardrails":{"prompt_injection":false,"pii":true}}}"#,
    "presidio",
    true
)]
#[case::every_callback_of_a_guardrail_applies(
    r#"{"metadata":{"guardrails":{"pii":true}}}"#,
    "pii_masking",
    true
)]
#[case::non_false_values_still_apply(
    r#"{"metadata":{"guardrails":{"pii":null,"prompt_injection":0}}}"#,
    "lakera_prompt_injection",
    true
)]
#[case::text_false_still_applies(
    r#"{"metadata":{"guardrails":{"pii":"false"}}}"#,
    "presidio",
    true
)]
#[case::empty_guardrails_dict_applies_nothing(
    r#"{"metadata":{"guardrails":{}}}"#,
    "presidio",
    false
)]
#[case::guardrail_without_callbacks(
    r#"{"metadata":{"guardrails":{"nothing":true}}}"#,
    "presidio",
    false
)]
#[case::no_metadata(r#"{"model":"m"}"#, "presidio", true)]
#[case::metadata_not_an_object(r#"{"metadata":"x"}"#, "presidio", true)]
#[case::metadata_null(r#"{"metadata":null}"#, "presidio", true)]
#[case::no_guardrails_key(r#"{"metadata":{"user":"u"}}"#, "presidio", true)]
#[case::guardrails_not_a_dict(r#"{"metadata":{"guardrails":["pii"]}}"#, "presidio", true)]
#[case::guardrails_null(r#"{"metadata":{"guardrails":null}}"#, "presidio", true)]
#[case::guardrails_false(r#"{"metadata":{"guardrails":false}}"#, "presidio", true)]
fn metadata_picks_the_guardrails_that_run(
    #[case] data: &str,
    #[case] guardrail_name: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        guardrail_applies(&object(data), guardrail_name, &callback_names()),
        Ok(expected)
    );
}

#[test]
fn a_requested_guardrail_nobody_configured_is_an_error() {
    let data = object(r#"{"metadata":{"guardrails":{"mystery":true}}}"#);
    assert_eq!(
        guardrail_applies(&data, "presidio", &callback_names()),
        Err(SelectionError::UnknownGuardrail {
            guardrail_name: "mystery".to_owned()
        })
    );
}

#[test]
fn an_unknown_guardrail_set_to_false_is_never_looked_up() {
    let data = object(r#"{"metadata":{"guardrails":{"mystery":false}}}"#);
    assert_eq!(
        guardrail_applies(&data, "presidio", &callback_names()),
        Ok(false)
    );
}

#[test]
fn an_unknown_guardrail_is_an_error_even_when_the_callback_already_matched() {
    let data = object(r#"{"metadata":{"guardrails":{"pii":true,"zzz":true}}}"#);
    assert_eq!(
        guardrail_applies(&data, "presidio", &callback_names()),
        Err(SelectionError::UnknownGuardrail {
            guardrail_name: "zzz".to_owned()
        })
    );
}
