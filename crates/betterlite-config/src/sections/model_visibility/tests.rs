#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn loaded(source: &str) -> bool {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .model_visibility
    .model_list_healthy_only
}

fn loaded_from(source: &str, environment: &[(&str, &str)]) -> Result<bool, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .model_visibility
        .model_list_healthy_only)
}

#[rstest]
#[case::absent("model_list: []\n", false)]
#[case::true_value("general_settings:\n  model_list_healthy_only: true\n", true)]
#[case::false_value("general_settings:\n  model_list_healthy_only: false\n", false)]
#[case::quoted_true("general_settings:\n  model_list_healthy_only: \"true\"\n", false)]
#[case::quoted_false("general_settings:\n  model_list_healthy_only: \"false\"\n", false)]
#[case::number("general_settings:\n  model_list_healthy_only: 1\n", false)]
#[case::null("general_settings:\n  model_list_healthy_only: null\n", false)]
#[case::list("general_settings:\n  model_list_healthy_only: [true]\n", false)]
#[case::mapping(
    "general_settings:\n  model_list_healthy_only: {enabled: true}\n",
    false
)]
#[case::other_general_settings(
    "general_settings:\n  background_health_checks: true\n  model_list_healthy_only: true\n",
    true
)]
fn loads_health_filter(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(loaded(source), expected);
}

#[rstest]
#[case::quoted_true("general_settings:\n  model_list_healthy_only: \"true\"\n")]
#[case::quoted_yes("general_settings:\n  model_list_healthy_only: \"yes\"\n")]
#[case::number("general_settings:\n  model_list_healthy_only: 1\n")]
#[case::float("general_settings:\n  model_list_healthy_only: 0.0\n")]
#[case::null("general_settings:\n  model_list_healthy_only: null\n")]
#[case::empty_list("general_settings:\n  model_list_healthy_only: []\n")]
#[case::empty_mapping("general_settings:\n  model_list_healthy_only: {}\n")]
fn health_filter_nonbool(#[case] source: &str) {
    assert!(!loaded(source), "{source}");
}

#[rstest]
#[case::on("true", true)]
#[case::one("1", true)]
#[case::yes("yes", true)]
#[case::off("false", false)]
#[case::zero("0", false)]
#[case::empty_counts_as_unset("", false)]
fn loads_health_filter_from_the_environment(#[case] value: &str, #[case] expected: bool) {
    let source =
        "general_settings:\n  model_list_healthy_only: os.environ/MODEL_LIST_HEALTHY_ONLY\n";
    assert_eq!(
        loaded_from(source, &[("MODEL_LIST_HEALTHY_ONLY", value)]).unwrap(),
        expected
    );
}

#[test]
fn health_filter_reports_the_environment() {
    let source =
        "general_settings:\n  model_list_healthy_only: os.environ/MODEL_LIST_HEALTHY_ONLY\n";
    let missing = loaded_from(source, &[]).unwrap_err();
    assert!(matches!(
        &missing,
        ConfigError::MissingEnvironment { variable, .. } if variable == "MODEL_LIST_HEALTHY_ONLY"
    ));
    let unreadable = loaded_from(source, &[("MODEL_LIST_HEALTHY_ONLY", "sometimes")]).unwrap_err();
    assert_eq!(
        unreadable.to_string(),
        "environment variable MODEL_LIST_HEALTHY_ONLY must be a bool"
    );
}
