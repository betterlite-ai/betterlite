#![cfg(test)]

use std::path::Path;

use rstest::rstest;

use crate::OracleError;
use crate::case::decode_case;

const TEMPLATE: &str = "\
name: release-info-fixture
config: |
  general_settings:
    master_key: oracle-master
{ENV}latest_release_fixture: upstream
request:
  method: GET
  path: /get/latest_release_info
  headers: {}
  body: null
upstream: []
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
";

fn case_text(env: &str) -> String {
    TEMPLATE.replace("{ENV}", env)
}

#[test]
fn binds_the_mock_destination_for_the_release_service() {
    let case = decode_case(Path::new("<fixture>"), case_text("env: {}\n").as_bytes())
        .expect("the fixture case should decode");
    assert_eq!(
        case.env.get(super::STARTUP_VARIABLE).map(String::as_str),
        Some("{{upstream}}")
    );
    assert_eq!(
        case.env
            .get(super::STARTUP_HOOKS_VARIABLE)
            .map(String::as_str),
        Some("startup:set_release_url")
    );
    assert_eq!(
        case.env
            .get(super::PYTHON_PATH_VARIABLE)
            .map(String::as_str),
        Some(super::FIXTURE_DIRECTORY)
    );
    assert!(
        case.config.ends_with(
            "environment_variables:\n  BETTERLITE_LATEST_RELEASE_URL: \"{{upstream}}\"\n"
        )
    );
}

#[test]
fn a_case_without_the_option_keeps_its_environment() {
    let text = TEMPLATE
        .replace("{ENV}", "env: {}\n")
        .replace("latest_release_fixture: upstream\n", "");
    let case =
        decode_case(Path::new("<plain>"), text.as_bytes()).expect("the plain case should decode");
    assert!(case.env.is_empty());
    assert!(!case.config.contains(super::DESTINATION_VARIABLE));
}

#[rstest]
#[case::startup_hooks("env: {LITELLM_WORKER_STARTUP_HOOKS: \"startup:other\"}\n")]
#[case::python_path("env: {PYTHONPATH: \"/elsewhere\"}\n")]
#[case::destination("env: {RELEASE_ORACLE_URL: \"http://elsewhere\"}\n")]
fn a_conflicting_environment_is_rejected(#[case] env: &str) {
    let outcome = decode_case(Path::new("<conflict>"), case_text(env).as_bytes());
    assert!(matches!(outcome, Err(OracleError::CaseInvalid { .. })));
}

#[test]
fn a_native_feature_case_is_rejected() {
    let text = format!(
        "{}native_feature_eligibility: true\n",
        case_text("env: {}\n")
    );
    let outcome = decode_case(Path::new("<native>"), text.as_bytes());
    assert!(matches!(outcome, Err(OracleError::CaseInvalid { .. })));
}

#[test]
fn a_config_destination_is_rejected() {
    let text = case_text("env: {}\n").replace(
        "    master_key: oracle-master",
        "    master_key: oracle-master\n  environment_variables:\n    BETTERLITE_LATEST_RELEASE_URL: http://elsewhere",
    );
    let outcome = decode_case(Path::new("<config>"), text.as_bytes());
    assert!(matches!(outcome, Err(OracleError::CaseInvalid { .. })));
}

#[test]
fn an_unknown_destination_is_rejected() {
    let text = case_text("env: {}\n").replace(
        "latest_release_fixture: upstream",
        "latest_release_fixture: elsewhere",
    );
    let outcome = decode_case(Path::new("<unknown>"), text.as_bytes());
    assert!(matches!(outcome, Err(OracleError::CaseInvalid { .. })));
}
