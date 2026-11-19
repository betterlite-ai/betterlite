#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::compression_interception_state::Settings;

fn lookup_of(environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn loaded(source: &str, environment: &[(&str, &str)]) -> Settings {
    let config = ConfigLoader {
        lookup: lookup_of(environment),
    }
    .load(source)
    .expect("the config loads");
    config.sections.compression_interception_state
}

fn load_error(source: &str, environment: &[(&str, &str)]) -> ConfigError {
    ConfigLoader {
        lookup: lookup_of(environment),
    }
    .load(source)
    .expect_err("the config fails to load")
}

fn expected(
    enabled: bool,
    compression_trigger: usize,
    compression_target: Option<usize>,
    embedding_model: Option<&str>,
    embedding_model_params: Option<IndexMap<String, Json>>,
) -> Settings {
    Settings {
        enabled,
        compression_trigger,
        compression_target,
        embedding_model: embedding_model.map(str::to_owned),
        embedding_model_params,
    }
}

fn embedding_params() -> Option<IndexMap<String, Json>> {
    Some(IndexMap::from([(
        "dimensions".to_owned(),
        Json::Number(Number::from(512_u16)),
    )]))
}

fn full_params(trigger: usize) -> String {
    format!(
        "    enabled: false\n    compression_trigger: {trigger}\n    compression_target: 700\n    embedding_model: text-embedding-3-small\n    embedding_model_params:\n      dimensions: 512\n"
    )
}

const NO_ENVIRONMENT: &[(&str, &str)] = &[];

#[rstest]
#[case::no_settings("litellm_settings: {}\n", expected(true, 200_000, None, None, None))]
#[case::empty_params(
    "litellm_settings:\n  compression_interception_params: {}\n",
    expected(true, 200_000, None, None, None)
)]
#[case::null_params(
    "litellm_settings:\n  compression_interception_params: null\n",
    expected(true, 200_000, None, None, None)
)]
#[case::null_enabled(
    "litellm_settings:\n  compression_interception_params:\n    enabled: null\n",
    expected(true, 200_000, None, None, None)
)]
#[case::trigger_zero(
    "litellm_settings:\n  compression_interception_params:\n    compression_trigger: 0\n",
    expected(true, 0, None, None, None)
)]
#[case::target_zero(
    "litellm_settings:\n  compression_interception_params:\n    compression_target: 0\n",
    expected(true, 200_000, Some(0), None, None)
)]
#[case::full_params(
    &full_params(1234),
    expected(false, 1234, Some(700), Some("text-embedding-3-small"), embedding_params())
)]
#[case::callback_fallback(
    "callback_settings:\n  compression_interception:\n    enabled: false\n    compression_trigger: 1234\n    compression_target: 700\n    embedding_model: text-embedding-3-small\n    embedding_model_params:\n      dimensions: 512\n",
    expected(
        false,
        1234,
        Some(700),
        Some("text-embedding-3-small"),
        embedding_params()
    )
)]
#[case::top_level_wins(
    "litellm_settings:\n  compression_interception_params:\n    compression_trigger: 1234\ncallback_settings:\n  compression_interception:\n    compression_trigger: 99\n",
    expected(true, 1234, None, None, None)
)]
#[case::empty_top_level_wins(
    "litellm_settings:\n  compression_interception_params: {}\ncallback_settings:\n  compression_interception:\n    compression_trigger: 99\n",
    expected(true, 200_000, None, None, None)
)]
#[case::null_callback_entry(
    "callback_settings:\n  compression_interception: null\n",
    expected(true, 200_000, None, None, None)
)]
fn loads_settings(#[case] source: &str, #[case] expectation: Settings) {
    assert_eq!(loaded(source, NO_ENVIRONMENT), expectation);
}

#[rstest]
#[case::enabled_yes("yes", true)]
#[case::enabled_off("off", false)]
#[case::enabled_one("1", true)]
#[case::enabled_zero("0", false)]
#[case::enabled_case("TRUE", true)]
#[case::enabled_empty("", true)]
fn loads_enabled_from_the_environment(#[case] value: &str, #[case] enabled: bool) {
    let source = "litellm_settings:\n  compression_interception_params:\n    enabled: os.environ/COMPRESSION_ENABLED\n";
    let environment = [("COMPRESSION_ENABLED", value)];
    assert_eq!(
        loaded(source, &environment),
        expected(enabled, 200_000, None, None, None)
    );
}

#[rstest]
#[case::whole_number("4096", 4096)]
#[case::padded(" 4096 ", 4096)]
#[case::empty("", 200_000)]
fn loads_the_trigger_from_the_environment(#[case] value: &str, #[case] trigger: usize) {
    let source = "litellm_settings:\n  compression_interception_params:\n    compression_trigger: os.environ/COMPRESSION_TRIGGER\n";
    let environment = [("COMPRESSION_TRIGGER", value)];
    assert_eq!(
        loaded(source, &environment),
        expected(true, trigger, None, None, None)
    );
}

#[rstest]
#[case::from_the_environment("os.environ/EMBEDDING_MODEL", Some("bedrock-titan"))]
#[case::empty_environment("os.environ/EMPTY_MODEL", None)]
#[case::plain_text("text-embedding-3-small", Some("text-embedding-3-small"))]
#[case::empty_text("\"\"", Some(""))]
fn loads_the_embedding_model(#[case] value: &str, #[case] model: Option<&str>) {
    let source = format!(
        "litellm_settings:\n  compression_interception_params:\n    embedding_model: {value}\n"
    );
    let environment = [("EMBEDDING_MODEL", "bedrock-titan"), ("EMPTY_MODEL", "")];
    assert_eq!(
        loaded(&source, &environment),
        expected(true, 200_000, None, model, None)
    );
}

#[test]
fn a_negative_trigger_is_a_load_error() {
    let source =
        "litellm_settings:\n  compression_interception_params:\n    compression_trigger: -5\n";
    assert!(matches!(
        load_error(source, NO_ENVIRONMENT),
        ConfigError::InvalidInteger { .. }
    ));
}

#[test]
fn a_fractional_trigger_is_a_load_error() {
    let source =
        "litellm_settings:\n  compression_interception_params:\n    compression_trigger: 1.5\n";
    assert!(matches!(
        load_error(source, NO_ENVIRONMENT),
        ConfigError::InvalidInteger { .. }
    ));
}

#[test]
fn a_negative_target_is_a_load_error() {
    let source =
        "litellm_settings:\n  compression_interception_params:\n    compression_target: -1\n";
    assert!(matches!(
        load_error(source, NO_ENVIRONMENT),
        ConfigError::InvalidInteger { .. }
    ));
}

#[test]
fn an_integer_enabled_is_a_load_error() {
    let source = "litellm_settings:\n  compression_interception_params:\n    enabled: 1\n";
    assert!(matches!(
        load_error(source, NO_ENVIRONMENT),
        ConfigError::Yaml { .. }
    ));
}

#[test]
fn an_unset_trigger_variable_is_a_load_error() {
    let source = "litellm_settings:\n  compression_interception_params:\n    compression_trigger: os.environ/MISSING_TRIGGER\n";
    assert!(matches!(
        load_error(source, NO_ENVIRONMENT),
        ConfigError::MissingEnvironment { .. }
    ));
}

#[test]
fn an_unknown_parameter_is_unsupported() {
    let source = "litellm_settings:\n  compression_interception_params:\n    unknown_key: 1\n";
    assert!(matches!(
        load_error(source, NO_ENVIRONMENT),
        ConfigError::Unsupported { .. }
    ));
}
