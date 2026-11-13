#![cfg(test)]

use rstest::rstest;

use crate::database_diagnostics::json_enabled::{
    GlobalJsonLogs, environment_enabled, is_json_enabled,
};
use crate::database_diagnostics::message::LogValue;

#[rstest]
#[case::bool_true(LogValue::Bool(true), Some("false"), true)]
#[case::bool_false(LogValue::Bool(false), Some("true"), false)]
#[case::text_false_is_truthy(LogValue::Text("false".to_owned()), Some("false"), true)]
#[case::zero_is_false(LogValue::Integer(0), Some("true"), false)]
#[case::empty_list_is_false(LogValue::List(Vec::new()), None, false)]
fn global_flag_precedes_env(
    #[case] global: LogValue,
    #[case] environment: Option<&str>,
    #[case] expected: bool,
) {
    let enabled = is_json_enabled(&GlobalJsonLogs::Supplied(global), environment).unwrap();
    assert_eq!(enabled, expected);
}

#[rstest]
#[case::env_true(Some("true"))]
#[case::env_absent(None)]
fn missing_global_attribute_false(#[case] environment: Option<&str>) {
    assert!(!is_json_enabled(&GlobalJsonLogs::Missing, environment).unwrap());
}

#[rstest]
#[case::import_error_true(GlobalJsonLogs::ImportError, Some("true"), true)]
#[case::import_error_false(GlobalJsonLogs::ImportError, Some("false"), false)]
#[case::attribute_error_true(GlobalJsonLogs::AttributeError, Some("TRUE"), true)]
#[case::attribute_error_absent(GlobalJsonLogs::AttributeError, None, false)]
fn only_import_attribute_errors_fallback(
    #[case] global: GlobalJsonLogs,
    #[case] environment: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(is_json_enabled(&global, environment).unwrap(), expected);
    assert!(is_json_enabled(&GlobalJsonLogs::Failed, environment).is_err());
}

#[rstest]
#[case::lowercase("true", true)]
#[case::uppercase("TRUE", true)]
#[case::mixed_case("tRuE", true)]
#[case::leading_space(" true", false)]
#[case::trailing_space("true ", false)]
#[case::empty("", false)]
#[case::one("1", false)]
#[case::yes("yes", false)]
fn env_true_casefold_without_trim(#[case] environment: &str, #[case] expected: bool) {
    assert_eq!(environment_enabled(Some(environment)), expected);
}

#[test]
fn global_false_ignores_true_env() {
    assert!(
        !is_json_enabled(
            &GlobalJsonLogs::Supplied(LogValue::Bool(false)),
            Some("true")
        )
        .unwrap()
    );
    assert!(
        is_json_enabled(
            &GlobalJsonLogs::Supplied(LogValue::Bool(true)),
            Some("false")
        )
        .unwrap()
    );
}
