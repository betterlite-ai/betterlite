#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::summary_model_access::{DEFAULT_SUMMARY_MAX_TOKENS, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .unwrap()
        .sections
        .summary_model_access
}

// the loader rejects a general_settings key no section claims, so every case here proves the key is supported
#[rstest]
#[case::absent("general_settings: {}\n", None, DEFAULT_SUMMARY_MAX_TOKENS)]
#[case::null_section("general_settings: null\n", None, DEFAULT_SUMMARY_MAX_TOKENS)]
#[case::explicit_null(
    "general_settings:\n  context_management_summary_model: null\n",
    None,
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::empty(
    "general_settings:\n  context_management_summary_model: \"\"\n",
    None,
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::numeric(
    "general_settings:\n  context_management_summary_model: 7\n",
    None,
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::whitespace(
    "general_settings:\n  context_management_summary_model: \" \"\n",
    Some(" "),
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::model(
    "general_settings:\n  context_management_summary_model: openai/gpt-4o-mini\n",
    Some("openai/gpt-4o-mini"),
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::zero(
    "general_settings:\n  context_management_summary_max_tokens: 0\n",
    None,
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::negative(
    "general_settings:\n  context_management_summary_max_tokens: -1\n",
    None,
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::float(
    "general_settings:\n  context_management_summary_max_tokens: 1.5\n",
    None,
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::numeric_string(
    "general_settings:\n  context_management_summary_max_tokens: \"2\"\n",
    None,
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::bool_false(
    "general_settings:\n  context_management_summary_max_tokens: false\n",
    None,
    DEFAULT_SUMMARY_MAX_TOKENS
)]
#[case::two(
    "general_settings:\n  context_management_summary_max_tokens: 2\n",
    None,
    2
)]
#[case::true_is_one(
    "general_settings:\n  context_management_summary_max_tokens: true\n",
    None,
    1
)]
#[case::both_keys(
    "general_settings:\n  context_management_summary_model: sum\n  \
     context_management_summary_max_tokens: 512\n",
    Some("sum"),
    512
)]
fn loads_summary_settings(
    #[case] source: &str,
    #[case] model: Option<&str>,
    #[case] max_tokens: u64,
) {
    let loaded = settings(source, &[]);
    assert_eq!(loaded.context_management_summary_model.as_deref(), model);
    assert_eq!(loaded.context_management_summary_max_tokens, max_tokens);
}

#[test]
fn load_summary_settings_ignores_default_max_tokens() {
    // DEFAULT_MAX_TOKENS is litellm's global completion cap, never the summary cap
    let loaded = settings(
        "general_settings:\n  context_management_summary_max_tokens: -1\n",
        &[("DEFAULT_MAX_TOKENS", "17")],
    );
    assert_eq!(
        loaded.context_management_summary_max_tokens,
        DEFAULT_SUMMARY_MAX_TOKENS
    );
}
