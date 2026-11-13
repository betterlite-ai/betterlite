#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{
    DEFAULT_MAX_PROMPT_LENGTH, MissingSessionId, Settings, SpendUpdates,
    decode_spend_storage_policy, load_spend_storage_policy, runtime_max_prompt_length,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(source: &str, pairs: &[(&str, &str)]) -> Settings {
    load_spend_storage_policy(source, &environment(pairs)).unwrap()
}

#[test]
fn loads_spend_storage_policy() {
    let initial = load(
        "general_settings:\n  store_prompts_in_spend_logs: false\n  missing_session_id: omit\n",
        &[("MAX_STRING_LENGTH_PROMPT_IN_DB", "100")],
    );
    assert!(!initial.store_prompts_in_spend_logs);
    assert_eq!(initial.max_string_length_prompt_in_db, 100);
    assert_eq!(initial.missing_session_id, Some(MissingSessionId::Omit));
    assert_eq!(
        runtime_max_prompt_length(initial.max_string_length_prompt_in_db, Some("200")),
        200
    );
    assert_eq!(
        runtime_max_prompt_length(initial.max_string_length_prompt_in_db, None),
        100
    );
    assert_eq!(
        runtime_max_prompt_length(initial.max_string_length_prompt_in_db, Some("not-a-number")),
        100
    );
    assert_eq!(
        runtime_max_prompt_length(initial.max_string_length_prompt_in_db, Some("")),
        100
    );
    let defaults = load("general_settings: {}\n", &[]);
    assert_eq!(
        defaults.max_string_length_prompt_in_db,
        DEFAULT_MAX_PROMPT_LENGTH
    );
    assert_eq!(defaults.missing_session_id, None);
    assert_eq!(defaults.spend_updates, SpendUpdates::Allowed);
}

#[rstest]
#[case::boolean_true("true", SpendUpdates::Disabled)]
#[case::boolean_false("false", SpendUpdates::Allowed)]
#[case::string_true("\"true\"", SpendUpdates::Allowed)]
#[case::number_one("1", SpendUpdates::Allowed)]
#[case::number_zero("0", SpendUpdates::Allowed)]
#[case::null("null", SpendUpdates::Allowed)]
fn loads_spend_update_flag(#[case] value: &str, #[case] expected: SpendUpdates) {
    let source = format!("general_settings:\n  disable_spend_updates: {value}\n");
    let settings = load(&source, &[]);
    assert_eq!(settings.spend_updates, expected);
}

#[rstest]
#[case::config_true("store_prompts_in_spend_logs: true", None, true)]
#[case::config_false("store_prompts_in_spend_logs: false", None, false)]
#[case::config_string_true("store_prompts_in_spend_logs: \"TRUE\"", None, true)]
#[case::config_string_false("store_prompts_in_spend_logs: \"true-ish\"", None, false)]
#[case::config_number("store_prompts_in_spend_logs: 1", None, false)]
#[case::env_true("store_prompts_in_spend_logs: false", Some("true"), true)]
#[case::env_upper("store_prompts_in_spend_logs: false", Some("TRUE"), true)]
#[case::env_yes("store_prompts_in_spend_logs: false", Some("yes"), false)]
#[case::env_false("store_prompts_in_spend_logs: true", Some("false"), true)]
fn resolves_prompts_storage_from_config_or_environment(
    #[case] setting: &str,
    #[case] store_prompts: Option<&str>,
    #[case] expected: bool,
) {
    let source = format!("general_settings:\n  {setting}\n");
    let pairs: Vec<(&str, &str)> = store_prompts
        .map(|value| vec![("STORE_PROMPTS_IN_SPEND_LOGS", value)])
        .unwrap_or_default();
    assert_eq!(load(&source, &pairs).store_prompts_in_spend_logs, expected);
}

#[rstest]
#[case::absent("", None)]
#[case::null("missing_session_id: null", None)]
#[case::generate("missing_session_id: generate", Some(MissingSessionId::Generate))]
#[case::reject("missing_session_id: reject", Some(MissingSessionId::Reject))]
#[case::omit("missing_session_id: omit", Some(MissingSessionId::Omit))]
fn parses_missing_session_policy(#[case] line: &str, #[case] expected: Option<MissingSessionId>) {
    let source = format!("general_settings:\n  {line}\n");
    let settings = load(&source, &[]);
    assert_eq!(settings.missing_session_id, expected);
}

#[rstest]
#[case::unknown("ignore")]
#[case::wrong_case("Omit")]
#[case::number("5")]
fn an_unknown_missing_session_policy_fails_loading(#[case] value: &str) {
    let source = format!("general_settings:\n  missing_session_id: {value}\n");
    let error = load_spend_storage_policy(&source, &environment(&[])).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("general_settings.missing_session_id")
    );
}

#[rstest]
#[case::text("not-a-number")]
#[case::fraction("20.5")]
#[case::python_digit_group("1_0")]
fn a_malformed_initial_prompt_length_fails_loading(#[case] value: &str) {
    let source = "general_settings: {}\n";
    let lookup = environment(&[("MAX_STRING_LENGTH_PROMPT_IN_DB", value)]);
    load_spend_storage_policy(source, &lookup).unwrap_err();
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative("-5", -5)]
#[case::surrounding_space(" 42 ", 42)]
fn keeps_zero_and_negative_prompt_limits(#[case] value: &str, #[case] expected: i64) {
    let source = "general_settings: {}\n";
    let lookup = environment(&[("MAX_STRING_LENGTH_PROMPT_IN_DB", value)]);
    assert_eq!(
        load_spend_storage_policy(source, &lookup)
            .unwrap()
            .max_string_length_prompt_in_db,
        expected
    );
}

#[rstest]
fn replays_the_spend_storage_policy_corpus(
    #[files("fuzz/corpus/spend_storage_policy/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _policy = decode_spend_storage_policy(&bytes);
}

#[test]
fn decodes_a_stored_policy_row() {
    let decoded = decode_spend_storage_policy(
        br#"{"general_settings":{"store_prompts_in_spend_logs":"true","disable_spend_updates":true},"environment":{"MAX_STRING_LENGTH_PROMPT_IN_DB":"12"}}"#,
    )
    .unwrap();
    assert!(decoded.store_prompts_in_spend_logs);
    assert_eq!(decoded.spend_updates, SpendUpdates::Disabled);
    assert_eq!(decoded.max_string_length_prompt_in_db, 12);
    decode_spend_storage_policy(b"not json").unwrap_err();
}
