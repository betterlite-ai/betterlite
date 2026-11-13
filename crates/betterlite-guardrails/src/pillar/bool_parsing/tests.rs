#![cfg(test)]

use betterlite_config::loader::EnvironmentLookup;
use rstest::rstest;

use super::bool_config::{BoolSetting, parse_bool_value, resolve_bool_config};
use super::error::PillarError;
use super::settings::{FallbackAction, OnFlaggedAction};

fn environment(entries: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn text(value: &str) -> BoolSetting {
    BoolSetting::Text(value.to_owned())
}

#[rstest]
#[case::true_word(text("true"))]
#[case::true_with_padding_and_case(text("  TRUE "))]
#[case::one(text("1"))]
#[case::yes(text("yes"))]
#[case::y(text("y"))]
#[case::on(text("on"))]
#[case::boolean_true(BoolSetting::Boolean(true))]
#[case::integer_one(BoolSetting::Integer(1))]
#[case::integer_negative(BoolSetting::Integer(-3))]
fn parses_true_values(#[case] value: BoolSetting) {
    assert_eq!(parse_bool_value(&value), Ok(true));
}

#[rstest]
#[case::false_word(text("false"))]
#[case::false_with_case(text("False"))]
#[case::zero_text(text("0"))]
#[case::no(text("no"))]
#[case::n(text("n"))]
#[case::off(text("off"))]
#[case::boolean_false(BoolSetting::Boolean(false))]
#[case::integer_zero(BoolSetting::Integer(0))]
fn parses_false_values(#[case] value: BoolSetting) {
    assert_eq!(parse_bool_value(&value), Ok(false));
}

#[rstest]
#[case::empty(text(""))]
#[case::word(text("maybe"))]
#[case::number(text("2"))]
#[case::dash(text("-"))]
fn rejects_unrecognised_values(#[case] value: BoolSetting) {
    let error = parse_bool_value(&value).expect_err("an unrecognised value is rejected");
    let BoolSetting::Text(raw) = &value else {
        panic!("the case is text");
    };
    assert!(matches!(&error, PillarError::BoolValue { value } if value == raw));
    assert_eq!(
        error.to_string(),
        format!("Unrecognised boolean value: {raw}")
    );
}

#[test]
fn an_explicit_value_beats_the_environment_and_the_default() {
    let env = environment(&[("PILLAR_ASYNC", "true")]);
    let provided = BoolSetting::Boolean(false);
    assert_eq!(
        resolve_bool_config(
            Some(&provided),
            Some("PILLAR_ASYNC"),
            Some(true),
            &env,
            "async_mode"
        ),
        Some(false)
    );
}

#[test]
fn an_explicit_zero_overrides_the_environment() {
    let env = environment(&[("PILLAR_ASYNC", "true")]);
    let provided = BoolSetting::Integer(0);
    assert_eq!(
        resolve_bool_config(
            Some(&provided),
            Some("PILLAR_ASYNC"),
            Some(true),
            &env,
            "async_mode"
        ),
        Some(false)
    );
}

#[test]
fn the_environment_beats_the_default_when_no_value_is_provided() {
    let env = environment(&[("PILLAR_INCLUDE_SCANNERS", "off")]);
    assert_eq!(
        resolve_bool_config(
            None,
            Some("PILLAR_INCLUDE_SCANNERS"),
            Some(true),
            &env,
            "include_scanners"
        ),
        Some(false)
    );
}

#[test]
fn the_default_applies_when_the_variable_is_unset() {
    let env = environment(&[]);
    assert_eq!(
        resolve_bool_config(None, Some("PILLAR_ASYNC"), None, &env, "async_mode"),
        None
    );
    assert_eq!(
        resolve_bool_config(
            None,
            Some("PILLAR_INCLUDE_EVIDENCE"),
            Some(true),
            &env,
            "include_evidence"
        ),
        Some(true)
    );
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let env = environment(&[("PILLAR_ASYNC", "")]);
    assert_eq!(
        resolve_bool_config(None, Some("PILLAR_ASYNC"), Some(true), &env, "async_mode"),
        Some(true)
    );
}

#[test]
fn an_invalid_environment_value_falls_back_to_the_default() {
    let env = environment(&[("PILLAR_ASYNC", "sometimes")]);
    assert_eq!(
        resolve_bool_config(None, Some("PILLAR_ASYNC"), Some(false), &env, "async_mode"),
        Some(false)
    );
}

#[test]
fn an_invalid_provided_value_falls_back_to_the_default() {
    let env = environment(&[("PILLAR_ASYNC", "true")]);
    let provided = text("sometimes");
    assert_eq!(
        resolve_bool_config(
            Some(&provided),
            Some("PILLAR_ASYNC"),
            Some(false),
            &env,
            "async_mode"
        ),
        Some(false)
    );
}

#[test]
fn no_environment_variable_name_reads_the_default() {
    let env = environment(&[]);
    assert_eq!(
        resolve_bool_config(None, None, Some(true), &env, "async_mode"),
        Some(true)
    );
}

#[test]
fn a_lookup_is_used_rather_than_the_process_environment() {
    let env = environment(&[("PILLAR_PERSIST", "1")]);
    assert_eq!(
        resolve_bool_config(None, Some("PILLAR_PERSIST"), None, &env, "persist_session"),
        Some(true)
    );
    assert_eq!(env.lookup("PILLAR_PERSIST"), Some("1".to_owned()));
}

#[test]
fn the_flagged_action_parses_its_supported_words() {
    assert_eq!(
        OnFlaggedAction::parse("block"),
        Some(OnFlaggedAction::Block)
    );
    assert_eq!(
        OnFlaggedAction::parse("monitor"),
        Some(OnFlaggedAction::Monitor)
    );
    assert_eq!(OnFlaggedAction::parse("mask"), Some(OnFlaggedAction::Mask));
    assert_eq!(OnFlaggedAction::parse("drop"), None);
    assert_eq!(OnFlaggedAction::parse(""), None);
}

#[test]
fn the_fallback_action_parses_its_supported_words() {
    assert_eq!(FallbackAction::parse("allow"), Some(FallbackAction::Allow));
    assert_eq!(FallbackAction::parse("block"), Some(FallbackAction::Block));
    assert_eq!(FallbackAction::parse("drop"), None);
    assert_eq!(FallbackAction::parse(""), None);
}
