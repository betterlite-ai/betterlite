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
    let environment = load(source, &[("CHECK_PROVIDER_ENDPOINT", "sometimes")]).unwrap_err();
    assert!(matches!(environment, ConfigError::Environment { .. }));
    assert_eq!(
        environment.to_string(),
        "environment variable CHECK_PROVIDER_ENDPOINT must be a bool"
    );
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::MissingEnvironment { .. })
    ));
    assert!(matches!(
        load("litellm_settings:\n  check_provider_endpoint: 1\n", &[]),
        Err(ConfigError::Yaml { .. })
    ));
    let inline = load(
        "litellm_settings:\n  check_provider_endpoint: sometimes\n",
        &[],
    )
    .unwrap_err();
    assert!(matches!(inline, ConfigError::Environment { .. }));
    assert_eq!(
        inline.to_string(),
        "environment variable litellm_settings.check_provider_endpoint must be a bool"
    );
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
