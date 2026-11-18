#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::ConfigLoader;

fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn source(global: Option<bool>) -> String {
    global.map_or_else(
        || "model_list: []\n".to_owned(),
        |value| format!("model_list: []\nlitellm_settings:\n  reasoning_auto_summary: {value}\n"),
    )
}

fn settings(global: Option<bool>, environment: &BTreeMap<String, String>) -> Settings {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source(global))
    .expect("the config loads")
    .sections
    .messages_thinking
}

#[rstest]
#[case::unset(&[], None, false)]
#[case::global_true(&[], Some(true), true)]
#[case::global_false(&[], Some(false), false)]
#[case::env_true(&[("LITELLM_REASONING_AUTO_SUMMARY", "true")], None, true)]
#[case::env_upper_true(&[("LITELLM_REASONING_AUTO_SUMMARY", "TRUE")], None, true)]
#[case::env_one(&[("LITELLM_REASONING_AUTO_SUMMARY", "1")], None, false)]
#[case::env_yes(&[("LITELLM_REASONING_AUTO_SUMMARY", "yes")], None, false)]
#[case::env_padded(&[("LITELLM_REASONING_AUTO_SUMMARY", " true ")], None, false)]
#[case::global_true_env_false(
    &[("LITELLM_REASONING_AUTO_SUMMARY", "false")],
    Some(true),
    true
)]
#[case::global_false_env_upper_true(
    &[("LITELLM_REASONING_AUTO_SUMMARY", "TRUE")],
    Some(false),
    true
)]
fn loads_summary_settings(
    #[case] environment: &[(&str, &str)],
    #[case] global: Option<bool>,
    #[case] expected: bool,
) {
    let loaded = settings(global, &variables(environment));
    assert_eq!(loaded.reasoning_auto_summary, expected);
}
