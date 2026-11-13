#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::database_diagnostics::initialization::DiagnosticSettings;
use betterlite_db::database_diagnostics::level::{LogLevel, parse_level};
use rstest::rstest;

use crate::loader::ConfigLoader;

fn convert(environment: &[(&str, &str)]) -> DiagnosticSettings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("{}")
        .unwrap()
        .sections
        .database_diagnostics
}

#[test]
fn tool_level_default_info() {
    let settings = convert(&[]);
    assert_eq!(settings.level, "INFO");
    assert!(!settings.json);
}

#[rstest]
#[case::lowercase(&[("LITELLM_LOG", "debug")], LogLevel::Debug)]
#[case::capitalized(&[("LITELLM_LOG", "Warn")], LogLevel::Warning)]
#[case::fatal(&[("LITELLM_LOG", "fatal")], LogLevel::Critical)]
#[case::notset(&[("LITELLM_LOG", "NoTsEt")], LogLevel::NotSet)]
#[case::info(&[("LITELLM_LOG", "info")], LogLevel::Info)]
#[case::warning(&[("LITELLM_LOG", "warning")], LogLevel::Warning)]
#[case::error(&[("LITELLM_LOG", "error")], LogLevel::Error)]
#[case::critical(&[("LITELLM_LOG", "critical")], LogLevel::Critical)]
fn tool_level_casefold_aliases(#[case] environment: &[(&str, &str)], #[case] expected: LogLevel) {
    let settings = convert(environment);
    assert_eq!(parse_level(&settings.level).unwrap(), expected);
}

#[rstest]
#[case::empty(&[("LITELLM_LOG", "")])]
#[case::numeric(&[("LITELLM_LOG", "10")])]
#[case::unknown(&[("LITELLM_LOG", "verbose")])]
#[case::untrimmed(&[("LITELLM_LOG", " info ")])]
fn tool_level_invalid_empty_numeric_fail(#[case] environment: &[(&str, &str)]) {
    let settings = convert(environment);
    assert!(parse_level(&settings.level).is_err());
}

#[rstest]
#[case::absent(&[], false)]
#[case::explicit_false(&[("JSON_LOGS", "false")], false)]
#[case::zero(&[("JSON_LOGS", "0")], false)]
fn json_env_default_false(#[case] environment: &[(&str, &str)], #[case] expected: bool) {
    assert_eq!(convert(environment).json, expected);
}

#[rstest]
#[case::lowercase(&[("JSON_LOGS", "true")], true)]
#[case::uppercase(&[("JSON_LOGS", "TRUE")], true)]
#[case::mixed_case(&[("JSON_LOGS", "tRuE")], true)]
#[case::leading_space(&[("JSON_LOGS", " true")], false)]
#[case::trailing_space(&[("JSON_LOGS", "true ")], false)]
#[case::one(&[("JSON_LOGS", "1")], false)]
#[case::yes(&[("JSON_LOGS", "yes")], false)]
fn json_env_exact_true(#[case] environment: &[(&str, &str)], #[case] expected: bool) {
    assert_eq!(convert(environment).json, expected);
}
