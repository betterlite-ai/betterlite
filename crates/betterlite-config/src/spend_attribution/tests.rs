#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{
    ENV_USE_LITELLM_PROXY, Settings, decode_spend_proxy_selection, load_spend_proxy_selection,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(source: &str, pairs: &[(&str, &str)]) -> Settings {
    load_spend_proxy_selection(source, &environment(pairs)).unwrap()
}

#[test]
fn loads_spend_proxy_selection() {
    let settings = load(
        "litellm_settings:\n  use_litellm_proxy: true\nmodel_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n      use_litellm_proxy: false\n  - model_name: b\n    litellm_params:\n      model: openai/gpt-4o\n  - model_name: c\n    litellm_params:\n      model: openai/gpt-4o\n      use_litellm_proxy: true\n",
        &[],
    );
    assert_eq!(
        settings,
        Settings {
            environment_use_litellm_proxy: false,
            global_use_litellm_proxy: true,
            deployment_use_litellm_proxy: vec![false, false, true],
        }
    );
}

#[rstest]
#[case::unset(None, false)]
#[case::true_value(Some("true"), true)]
#[case::upper_case_value(Some("TRUE"), true)]
#[case::one(Some("1"), true)]
#[case::false_value(Some("false"), false)]
#[case::zero(Some("0"), false)]
#[case::empty(Some(""), false)]
fn reads_the_environment_flag(#[case] value: Option<&str>, #[case] expected: bool) {
    let pairs: Vec<(&str, &str)> = value
        .map(|text| (ENV_USE_LITELLM_PROXY, text))
        .into_iter()
        .collect();
    let settings = load("litellm_settings: {}\n", &pairs);
    assert_eq!(settings.environment_use_litellm_proxy, expected);
}

#[test]
fn rejects_an_unparsable_environment_flag() {
    let error = load_spend_proxy_selection(
        "litellm_settings: {}\n",
        &environment(&[(ENV_USE_LITELLM_PROXY, "maybe")]),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "environment variable USE_LITELLM_PROXY must be a bool"
    );
}

#[rstest]
#[case::global(0)]
#[case::deployment(1)]
fn rejects_a_non_boolean_config_flag(#[case] position: u8) {
    let source = if position == 0 {
        "litellm_settings:\n  use_litellm_proxy: 1\n"
    } else {
        "model_list:\n  - litellm_params:\n      use_litellm_proxy: 1\n"
    };
    assert!(load_spend_proxy_selection(source, &environment(&[])).is_err());
}

#[test]
fn decodes_a_stored_selection_row() {
    let settings = decode_spend_proxy_selection(
        br#"{"litellm_settings":{"use_litellm_proxy":false},"model_list":[{"litellm_params":{"use_litellm_proxy":true}},{"litellm_params":{}}],"environment":{"USE_LITELLM_PROXY":"true"}}"#,
    )
    .unwrap();
    assert_eq!(
        settings,
        Settings {
            environment_use_litellm_proxy: true,
            global_use_litellm_proxy: false,
            deployment_use_litellm_proxy: vec![true, false],
        }
    );
}

#[rstest]
#[case::not_json(b"{".as_slice())]
#[case::not_an_object(b"[]".as_slice())]
#[case::row_has_wrong_params_type(br#"{"model_list":[{"litellm_params":"openai"}]}"#.as_slice())]
fn rejects_a_malformed_selection_row(#[case] bytes: &[u8]) {
    assert!(decode_spend_proxy_selection(bytes).is_err());
}

#[rstest]
fn replays_the_spend_proxy_selection_corpus(
    #[files("fuzz/corpus/spend_proxy_selection/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _settings = decode_spend_proxy_selection(&bytes);
}
