#![cfg(test)]

use std::{collections::BTreeMap, error, time::Duration};

use rstest::rstest;

use super::{Options, Settings};
use crate::google_secret_manager::GoogleSecretManagerError;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::unset(&[])]
#[case::empty(&[("GOOGLE_SECRET_MANAGER_PROJECT_ID", "")])]
fn missing_project_id_is_an_error(#[case] pairs: &[(&str, &str)]) {
    let error = Settings::resolve(&environment(pairs), Options::default()).unwrap_err();

    assert!(matches!(error, GoogleSecretManagerError::MissingProjectId));
}

#[test]
fn defaults_are_a_day_and_cached_reads() {
    let settings = Settings::resolve(
        &environment(&[("GOOGLE_SECRET_MANAGER_PROJECT_ID", "project-one")]),
        Options::default(),
    )
    .unwrap();

    assert_eq!(settings.project_id, "project-one");
    assert_eq!(settings.refresh_interval, Duration::from_hours(24));
    assert!(!settings.always_read_secret_manager);
}

#[rstest]
#[case::environment_beats_parameter(
    &[("GOOGLE_SECRET_MANAGER_REFRESH_INTERVAL", "30"), ("SECRET_MANAGER_REFRESH_INTERVAL", "90")],
    Some(60),
    30
)]
#[case::parameter_beats_shared_default(&[("SECRET_MANAGER_REFRESH_INTERVAL", "90")], Some(60), 60)]
#[case::shared_default(&[("SECRET_MANAGER_REFRESH_INTERVAL", "90")], None, 90)]
#[case::empty_environment_is_unset(&[("GOOGLE_SECRET_MANAGER_REFRESH_INTERVAL", "")], Some(60), 60)]
#[case::zero_is_used_as_given(&[("GOOGLE_SECRET_MANAGER_REFRESH_INTERVAL", "0")], Some(60), 0)]
fn refresh_interval_resolution(
    #[case] pairs: &[(&str, &str)],
    #[case] parameter: Option<u64>,
    #[case] expected_seconds: u64,
) {
    let mut variables = environment(pairs);
    variables.insert(
        "GOOGLE_SECRET_MANAGER_PROJECT_ID".to_owned(),
        "project-one".to_owned(),
    );

    let settings = Settings::resolve(
        &variables,
        Options {
            refresh_interval: parameter,
            always_read_secret_manager: None,
        },
    )
    .unwrap();

    assert_eq!(
        settings.refresh_interval,
        Duration::from_secs(expected_seconds)
    );
}

#[rstest]
#[case::word("GOOGLE_SECRET_MANAGER_REFRESH_INTERVAL", "soon")]
#[case::negative("GOOGLE_SECRET_MANAGER_REFRESH_INTERVAL", "-5")]
fn invalid_text_names_the_variable(#[case] name: &str, #[case] text: &str) {
    let variables = environment(&[
        ("GOOGLE_SECRET_MANAGER_PROJECT_ID", "project-one"),
        (name, text),
    ]);

    let error = Settings::resolve(&variables, Options::default()).unwrap_err();

    assert!(matches!(error, GoogleSecretManagerError::Environment(_)));
    assert!(
        error::Error::source(&error)
            .unwrap()
            .to_string()
            .contains(name)
    );
}

#[rstest]
#[case::lowercase(Some("true"), None, true)]
#[case::uppercase(Some("TRUE"), None, true)]
#[case::mixed_case(Some("True"), Some(false), true)]
#[case::false_text_keeps_parameter(Some("false"), Some(true), true)]
#[case::numeric_text_is_not_true(Some("1"), None, false)]
#[case::yes_text_is_not_true(Some("yes"), None, false)]
#[case::padded_text_is_not_true(Some(" true "), None, false)]
#[case::unknown_text_is_not_true(Some("maybe"), None, false)]
#[case::unknown_text_keeps_parameter(Some("maybe"), Some(true), true)]
#[case::empty_text_is_unset(Some(""), Some(true), true)]
#[case::false_text_without_parameter(Some("no"), None, false)]
#[case::unset_uses_parameter(None, Some(true), true)]
#[case::unset_and_no_parameter(None, None, false)]
fn always_read_resolution(
    #[case] text: Option<&str>,
    #[case] parameter: Option<bool>,
    #[case] expected: bool,
) {
    let mut variables = environment(&[("GOOGLE_SECRET_MANAGER_PROJECT_ID", "project-one")]);
    if let Some(value) = text {
        variables.insert(
            "GOOGLE_SECRET_MANAGER_ALWAYS_READ_SECRET_MANAGER".to_owned(),
            value.to_owned(),
        );
    }

    let settings = Settings::resolve(
        &variables,
        Options {
            refresh_interval: None,
            always_read_secret_manager: parameter,
        },
    )
    .unwrap();

    assert_eq!(settings.always_read_secret_manager, expected);
}
