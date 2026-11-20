#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use rust_decimal::Decimal;

use super::{
    DEFAULT_MAX_PROMPT_LENGTH, DEFAULT_MAX_REDIS_BUFFER_DEQUEUE_COUNT,
    ENV_MAX_REDIS_BUFFER_DEQUEUE_COUNT, MissingSessionId, Settings, SpendUpdates,
    decode_spend_storage_policy, load_spend_storage_policy, runtime_max_prompt_length,
    runtime_max_redis_buffer_dequeue_count,
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

#[test]
fn loads_redis_spend_settings() {
    let defaults = load("general_settings: {}\n", &[]);
    assert!(!defaults.use_redis_transaction_buffer);
    assert_eq!(
        defaults.max_redis_buffer_dequeue_count,
        DEFAULT_MAX_REDIS_BUFFER_DEQUEUE_COUNT
    );
    let enabled = load(
        "general_settings:\n  use_redis_transaction_buffer: true\n",
        &[(ENV_MAX_REDIS_BUFFER_DEQUEUE_COUNT, "250")],
    );
    assert!(enabled.use_redis_transaction_buffer);
    assert_eq!(enabled.max_redis_buffer_dequeue_count, 250);
    let initial = enabled.max_redis_buffer_dequeue_count;
    assert_eq!(
        runtime_max_redis_buffer_dequeue_count(initial, Some("10")),
        10
    );
    assert_eq!(runtime_max_redis_buffer_dequeue_count(initial, None), 250);
    assert_eq!(runtime_max_redis_buffer_dequeue_count(initial, Some("")), 250);
    assert_eq!(
        runtime_max_redis_buffer_dequeue_count(initial, Some("not-a-number")),
        250
    );
}

#[test]
fn loads_spend_summary_budget() {
    let configured = load("litellm_settings:\n  max_budget: 100.5\n", &[]);
    assert_eq!(configured.max_budget, Decimal::new(1005, 1));
    let from_environment = load(
        "litellm_settings:\n  max_budget: os.environ/PROXY_MAX_BUDGET\n",
        &[("PROXY_MAX_BUDGET", "12.25")],
    );
    assert_eq!(from_environment.max_budget, Decimal::new(1225, 2));
    let defaults = load("general_settings: {}\n", &[]);
    assert_eq!(defaults.max_budget, Decimal::ZERO);
    let error = load_spend_storage_policy(
        "litellm_settings:\n  max_budget: not-a-number\n",
        &environment(&[]),
    )
    .unwrap_err();
    assert!(error.to_string().contains("litellm_settings.max_budget"));
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
#[case::boolean_true("true", true)]
#[case::boolean_false("false", false)]
#[case::string_true("\"true\"", true)]
#[case::string_padded_upper("\" TRUE \"", true)]
#[case::string_false("\"false\"", false)]
#[case::unrecognized_string("\"yes\"", false)]
#[case::null("null", false)]
fn loads_redis_transaction_buffer_policy(#[case] value: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  use_redis_transaction_buffer: {value}\n");
    assert_eq!(load(&source, &[]).use_redis_transaction_buffer, expected);
}

#[rstest]
#[case::number("1")]
#[case::list("[true]")]
#[case::map("{}")]
fn rejects_a_redis_transaction_buffer_that_is_not_a_flag(#[case] value: &str) {
    let source = format!("general_settings:\n  use_redis_transaction_buffer: {value}\n");
    let error = load_spend_storage_policy(&source, &environment(&[])).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("general_settings.use_redis_transaction_buffer")
    );
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative("-5", -5)]
#[case::surrounding_space(" 42 ", 42)]
fn keeps_zero_and_negative_redis_dequeue_counts(#[case] value: &str, #[case] expected: i64) {
    let source = "general_settings: {}\n";
    let lookup = environment(&[(ENV_MAX_REDIS_BUFFER_DEQUEUE_COUNT, value)]);
    assert_eq!(
        load_spend_storage_policy(source, &lookup)
            .unwrap()
            .max_redis_buffer_dequeue_count,
        expected
    );
}

#[rstest]
#[case::text("many")]
#[case::fraction("20.5")]
#[case::python_digit_group("1_0")]
fn a_malformed_redis_dequeue_count_fails_loading(#[case] value: &str) {
    let source = "general_settings: {}\n";
    let lookup = environment(&[(ENV_MAX_REDIS_BUFFER_DEQUEUE_COUNT, value)]);
    let error = load_spend_storage_policy(source, &lookup).unwrap_err();
    assert!(error.to_string().contains(ENV_MAX_REDIS_BUFFER_DEQUEUE_COUNT));
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
        br#"{"general_settings":{"store_prompts_in_spend_logs":"true","disable_spend_updates":true},"litellm_settings":{"max_budget":7.5},"environment":{"MAX_STRING_LENGTH_PROMPT_IN_DB":"12"}}"#,
    )
    .unwrap();
    assert!(decoded.store_prompts_in_spend_logs);
    assert_eq!(decoded.spend_updates, SpendUpdates::Disabled);
    assert_eq!(decoded.max_string_length_prompt_in_db, 12);
    assert_eq!(decoded.max_budget, Decimal::new(75, 1));
    decode_spend_storage_policy(b"not json").unwrap_err();
}

#[rstest]
#[case::integer("100", Decimal::from(100_u8))]
#[case::float("7.5", Decimal::new(75, 1))]
#[case::text("\"12.25\"", Decimal::new(1225, 2))]
fn decodes_a_stored_budget(#[case] configured: &str, #[case] expected: Decimal) {
    let row = format!(r#"{{"litellm_settings":{{"max_budget":{configured}}},"environment":{{}}}}"#);
    let decoded = decode_spend_storage_policy(row.as_bytes()).unwrap();
    assert_eq!(decoded.max_budget, expected);
}

#[rstest]
#[case::boolean("true")]
#[case::array("[1]")]
#[case::object("{}")]
fn rejects_a_stored_budget_that_is_not_a_number(#[case] configured: &str) {
    let row = format!(r#"{{"litellm_settings":{{"max_budget":{configured}}},"environment":{{}}}}"#);
    let error = decode_spend_storage_policy(row.as_bytes()).unwrap_err();
    assert!(error.to_string().contains("litellm_settings.max_budget"));
}

#[rstest]
#[case::defaults("litellm_settings: {}\n", None, None)]
#[case::null_values(
    "litellm_settings:\n  disable_end_user_cost_tracking: null\n  enable_end_user_cost_tracking_prometheus_only: null\n",
    None,
    None
)]
#[case::disable_true(
    "litellm_settings:\n  disable_end_user_cost_tracking: true\n",
    Some(true),
    None
)]
#[case::disable_false(
    "litellm_settings:\n  disable_end_user_cost_tracking: false\n",
    Some(false),
    None
)]
#[case::prometheus_true(
    "litellm_settings:\n  enable_end_user_cost_tracking_prometheus_only: true\n",
    None,
    Some(true)
)]
#[case::prometheus_false(
    "litellm_settings:\n  enable_end_user_cost_tracking_prometheus_only: false\n",
    None,
    Some(false)
)]
#[case::both_set(
    "litellm_settings:\n  disable_end_user_cost_tracking: false\n  enable_end_user_cost_tracking_prometheus_only: true\n",
    Some(false),
    Some(true)
)]
fn loads_end_user_spend_settings(
    #[case] source: &str,
    #[case] disable: Option<bool>,
    #[case] prometheus_only: Option<bool>,
) {
    let settings = load(source, &[]);
    assert_eq!(settings.disable_end_user_cost_tracking, disable);
    assert_eq!(settings.enable_prometheus_only, prometheus_only);
}

#[rstest]
#[case::disable_string(
    "litellm_settings:\n  disable_end_user_cost_tracking: \"true\"\n",
    "litellm_settings.disable_end_user_cost_tracking"
)]
#[case::disable_number(
    "litellm_settings:\n  disable_end_user_cost_tracking: 1\n",
    "litellm_settings.disable_end_user_cost_tracking"
)]
#[case::prometheus_string(
    "litellm_settings:\n  enable_end_user_cost_tracking_prometheus_only: \"true\"\n",
    "litellm_settings.enable_end_user_cost_tracking_prometheus_only"
)]
#[case::prometheus_list(
    "litellm_settings:\n  enable_end_user_cost_tracking_prometheus_only: [true]\n",
    "litellm_settings.enable_end_user_cost_tracking_prometheus_only"
)]
fn loads_end_user_spend_settings_rejects_a_wrong_type(#[case] source: &str, #[case] path: &str) {
    let error = load_spend_storage_policy(source, &environment(&[])).unwrap_err();
    assert!(error.to_string().contains(path));
}

#[test]
fn loads_end_user_spend_settings_from_a_stored_policy_row() {
    let decoded = decode_spend_storage_policy(
        br#"{"litellm_settings":{"disable_end_user_cost_tracking":true,"enable_end_user_cost_tracking_prometheus_only":false},"environment":{}}"#,
    )
    .unwrap();
    assert_eq!(decoded.disable_end_user_cost_tracking, Some(true));
    assert_eq!(decoded.enable_prometheus_only, Some(false));
}
