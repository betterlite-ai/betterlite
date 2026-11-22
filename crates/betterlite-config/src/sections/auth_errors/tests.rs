#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::auth_errors::Settings;
use crate::types::Config;

fn load(source: &str) -> Result<Config, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
}

#[rstest]
#[case::empty("{}")]
#[case::absent("litellm_settings:\n  drop_params: true\n")]
#[case::explicit_null(
    "litellm_settings:\n  log_auth_failure_key_identity: null\n  log_client_error_tracebacks: null\n"
)]
fn log_switches_default_false(#[case] source: &str) {
    assert_eq!(
        load(source).unwrap().sections.auth_errors,
        Settings::default()
    );
    assert_eq!(
        Settings::default(),
        Settings {
            log_auth_failure_key_identity: false,
            log_client_error_tracebacks: false,
        }
    );
}

#[rstest]
#[case::identity_only("log_auth_failure_key_identity: true", true, false)]
#[case::tracebacks_only("log_client_error_tracebacks: true", false, true)]
#[case::both_off(
    "log_auth_failure_key_identity: false\n  log_client_error_tracebacks: false",
    false,
    false
)]
fn identity_and_traceback_switches_consumed(
    #[case] setting: &str,
    #[case] identity: bool,
    #[case] tracebacks: bool,
) {
    let config = load(&format!("litellm_settings:\n  {setting}\n")).unwrap();
    assert_eq!(
        config.sections.auth_errors,
        Settings {
            log_auth_failure_key_identity: identity,
            log_client_error_tracebacks: tracebacks,
        }
    );
}

#[rstest]
#[case::absent("master_key: sk-test", false)]
#[case::off("use_x_forwarded_for: false", false)]
#[case::on("use_x_forwarded_for: true", true)]
fn forwarded_for_requires_exact_true(#[case] setting: &str, #[case] expected: bool) {
    let config = load(&format!("general_settings:\n  {setting}\n")).unwrap();
    assert_eq!(
        config.sections.auth_request_policy.use_x_forwarded_for,
        expected
    );
    // text that is no switch at all is a load error rather than a silent true
    let text = load("general_settings:\n  use_x_forwarded_for: sometimes\n");
    assert!(matches!(text, Err(ConfigError::Yaml { .. })));
}

#[rstest]
#[case::absent("master_key: sk-test", AllowRequestsOnOutage::Boolean(false))]
#[case::enabled(
    "allow_requests_on_db_unavailable: true",
    AllowRequestsOnOutage::Boolean(true)
)]
fn outage_default_false_and_enabled(
    #[case] setting: &str,
    #[case] expected: AllowRequestsOnOutage,
) {
    let config = load(&format!("general_settings:\n  {setting}\n")).unwrap();
    assert_eq!(
        config.sections.database_availability.allow_requests,
        expected
    );
}

#[rstest]
#[case::identity_text("log_auth_failure_key_identity: yes-please")]
#[case::identity_number("log_auth_failure_key_identity: 1")]
#[case::tracebacks_list("log_client_error_tracebacks: [true]")]
fn invalid_boolean_rejected(#[case] setting: &str) {
    let outcome = load(&format!("litellm_settings:\n  {setting}\n"));
    assert!(
        matches!(outcome, Err(ConfigError::Yaml { .. })),
        "{outcome:?}"
    );
}
