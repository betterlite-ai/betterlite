use std::time::Duration;

use rstest::rstest;

use super::support::{
    AgentKind, Fixture, capture_logs, default_fixture, environment, event, payload,
};
use crate::callback_logger::CallbackLogger;
use crate::newrelic_agent::backoff::METRIC_WINDOW;
use crate::newrelic_agent::events::SUPPORTABILITY_METRIC_PREFIX;
use crate::newrelic_agent::init::NewRelicInit;

fn enabled_fixture() -> Fixture {
    default_fixture()
}

#[test]
fn credentials_from_the_environment_register_the_agent_with_a_zero_timeout() {
    let fixture = enabled_fixture();
    let (logger, logs) = capture_logs(|| fixture.build());
    assert!(logger.enabled);
    assert_eq!(fixture.registrations(), vec![Duration::ZERO]);
    assert!(logs.contains("New Relic AI Monitoring initialized"));
    assert!(logs.contains("my-app"));
}

#[rstest]
#[case::missing_license(None, Some("my-app"))]
#[case::missing_app(Some("license-key"), None)]
#[case::empty_license(Some(""), Some("my-app"))]
#[case::empty_app(Some("license-key"), Some(""))]
fn missing_credentials_disable_the_integration_with_a_warning(
    #[case] license_value: Option<&str>,
    #[case] app_value: Option<&str>,
) {
    let mut fixture = enabled_fixture();
    fixture.environment = match (license_value, app_value) {
        (Some(license), Some(app)) => environment(&[
            ("NEW_RELIC_LICENSE_KEY", license),
            ("NEW_RELIC_APP_NAME", app),
        ]),
        (Some(license), None) => environment(&[("NEW_RELIC_LICENSE_KEY", license)]),
        (None, Some(app)) => environment(&[("NEW_RELIC_APP_NAME", app)]),
        (None, None) => environment(&[]),
    };
    let (logger, logs) = capture_logs(|| fixture.build());
    assert!(!logger.enabled);
    assert!(fixture.registrations().is_empty());
    assert!(logs.contains("NEW_RELIC_LICENSE_KEY"));
    assert!(logs.contains("NEW_RELIC_APP_NAME"));
    assert!(logs.contains("Integration will be disabled"));
}

#[test]
fn missing_credentials_are_reported_before_a_missing_agent() {
    let mut fixture = enabled_fixture();
    fixture.agent_kind = AgentKind::Missing;
    fixture.environment = environment(&[]);
    let (logger, logs) = capture_logs(|| fixture.build());
    assert!(!logger.enabled);
    assert!(logs.contains("NEW_RELIC_LICENSE_KEY"));
    assert!(!logs.contains("not installed"));
}

#[test]
fn a_missing_agent_module_disables_the_integration_with_an_error() {
    let mut fixture = enabled_fixture();
    fixture.agent_kind = AgentKind::Missing;
    let (logger, logs) = capture_logs(|| fixture.build());
    assert!(!logger.enabled);
    assert!(logs.contains("ERROR"));
    assert!(logs.contains("New Relic Python agent not installed"));
}

#[test]
fn a_registration_failure_disables_the_integration_with_an_error() {
    let mut fixture = enabled_fixture();
    fixture.register_error = Some("registration exploded".to_owned());
    let (logger, logs) = capture_logs(|| fixture.build());
    assert!(!logger.enabled);
    assert!(logs.contains("ERROR"));
    assert!(logs.contains("registration exploded"));
    assert!(logs.contains("Integration will be disabled"));
}

#[tokio::test]
async fn a_disabled_integration_records_nothing() {
    let mut fixture = enabled_fixture();
    fixture.environment = environment(&[("NEW_RELIC_APP_NAME", "my-app")]);
    let logger = fixture.build();
    assert!(!logger.enabled);
    let callback: &dyn CallbackLogger = logger.as_ref();
    callback
        .async_log_success_event(event(payload()))
        .await
        .unwrap();
    callback
        .async_log_failure_event(event(payload()))
        .await
        .unwrap();
    assert!(fixture.application.events().is_empty());
    assert!(fixture.application.metrics().is_empty());
}

#[rstest]
#[case::constructor_wins_over_global(Some(false), true, false)]
#[case::constructor_true_over_global_false(Some(true), false, true)]
#[case::global_applies_without_a_constructor_value(None, true, true)]
#[case::both_default_to_false(None, false, false)]
fn constructor_params_override_the_global_newrelic_params(
    #[case] explicit: Option<bool>,
    #[case] global: bool,
    #[case] expected: bool,
) {
    let mut fixture = enabled_fixture();
    fixture.turn_off(global);
    let params = fixture.init.params;
    fixture.init = NewRelicInit {
        params,
        turn_off_message_logging: explicit,
    };
    let logger = fixture.build();
    assert_eq!(logger.settings.turn_off_message_logging, expected);
    let callback: &dyn CallbackLogger = logger.as_ref();
    assert_eq!(callback.turn_off_message_logging(), expected);
}

#[test]
fn constructor_turn_off_wins_over_the_global_params() {
    let mut fixture = enabled_fixture();
    fixture.turn_off(true);
    fixture.init.turn_off_message_logging = Some(false);
    let logger = fixture.build();
    assert!(!logger.settings.turn_off_message_logging);
}

#[test]
fn an_unavailable_version_becomes_unknown() {
    let mut fixture = enabled_fixture();
    fixture.version = None;
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    assert_eq!(
        fixture.application.supportability_metrics(),
        vec![format!("{SUPPORTABILITY_METRIC_PREFIX}unknown")]
    );
}

#[test]
fn a_supportability_metric_is_shared_across_loggers_and_emitted_once_per_window() {
    let fixture = enabled_fixture();
    let first = fixture.build();
    let second = fixture.build();
    let success = event(payload());
    first.log_success_event(&success).unwrap();
    assert_eq!(
        fixture.application.supportability_metrics(),
        vec![format!("{SUPPORTABILITY_METRIC_PREFIX}1.2.3")]
    );
    second.log_success_event(&success).unwrap();
    fixture.clock.advance(97_199);
    first.log_success_event(&success).unwrap();
    assert_eq!(fixture.application.supportability_metrics().len(), 1);
    fixture.clock.advance(1);
    second.log_success_event(&success).unwrap();
    assert_eq!(fixture.application.supportability_metrics().len(), 2);
}

#[test]
fn the_window_advances_even_when_the_application_is_not_enabled() {
    let fixture = enabled_fixture();
    fixture.application.set_enabled(false);
    let logger = fixture.build();
    assert!(logger.enabled);
    let success = event(payload());
    logger.log_success_event(&success).unwrap();
    assert!(fixture.application.supportability_metrics().is_empty());
    fixture.application.set_enabled(true);
    logger.log_success_event(&success).unwrap();
    assert!(fixture.application.supportability_metrics().is_empty());
    fixture.clock.advance(METRIC_WINDOW.as_secs());
    logger.log_success_event(&success).unwrap();
    assert_eq!(fixture.application.supportability_metrics().len(), 1);
}

#[rstest]
#[case::absent(None, "unknown")]
#[case::empty(Some(""), "unknown")]
#[case::present(Some("9.9.9"), "9.9.9")]
fn the_supportability_metric_names_the_package_version(
    #[case] version: Option<&str>,
    #[case] expected: &str,
) {
    let mut fixture = enabled_fixture();
    fixture.version = version.map(str::to_owned);
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    assert_eq!(
        fixture.application.supportability_metrics(),
        vec![format!("{SUPPORTABILITY_METRIC_PREFIX}{expected}")]
    );
}

#[test]
fn the_window_constant_is_twenty_seven_hours() {
    assert_eq!(METRIC_WINDOW, Duration::from_hours(27));
}
