#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .available_models)
}

#[rstest]
#[case::absent("", false)]
#[case::explicit_true("litellm_settings:\n  check_provider_endpoint: true\n", true)]
#[case::explicit_false("litellm_settings:\n  check_provider_endpoint: false\n", false)]
fn endpoint_checks_default_false(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(load(source, &[]).unwrap().check_provider_endpoint, expected);
}

#[rstest]
#[case::one("1", true)]
#[case::yes("yes", true)]
#[case::on_in_any_case("On", true)]
#[case::zero("0", false)]
#[case::no("no", false)]
#[case::empty_counts_as_unset("", false)]
fn endpoint_checks_read_the_environment(#[case] value: &str, #[case] expected: bool) {
    let source =
        "litellm_settings:\n  check_provider_endpoint: os.environ/CHECK_PROVIDER_ENDPOINT\n";
    assert_eq!(
        load(source, &[("CHECK_PROVIDER_ENDPOINT", value)])
            .unwrap()
            .check_provider_endpoint,
        expected
    );
}

#[test]
fn endpoint_checks_reject_an_unreadable_value() {
    let source =
        "litellm_settings:\n  check_provider_endpoint: os.environ/CHECK_PROVIDER_ENDPOINT\n";
    assert!(matches!(
        load(source, &[("CHECK_PROVIDER_ENDPOINT", "sometimes")]),
        Err(ConfigError::Environment { .. })
    ));
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::MissingEnvironment { .. })
    ));
    assert!(matches!(
        load("litellm_settings:\n  check_provider_endpoint: 1\n", &[]),
        Err(ConfigError::Yaml { .. })
    ));
}

#[rstest]
#[case::unrelated_litellm_setting("litellm_settings:\n  unrelated_available_models: true\n")]
#[case::deployment_setting(
    "model_list:\n- model_name: a\n  litellm_params:\n    check_provider_endpoint: true\n"
)]
fn rejects_unrelated_available_models_keys(#[case] source: &str) {
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}
