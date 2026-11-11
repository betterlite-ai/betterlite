use super::support::{AgentKind, attribute, default_fixture, environment, json, number_value};
use crate::exporter_options::health::HealthState;
use crate::newrelic_agent::health::{CONNECTION_TEST_EVENT_NAME, async_health_check};

#[test]
fn a_healthy_application_records_a_connection_test_event() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let status = async_health_check(&logger);
    assert_eq!(status.status, HealthState::Healthy);
    assert_eq!(status.error_message, None);
    let events = fixture.application.events_named(CONNECTION_TEST_EVENT_NAME);
    assert_eq!(events.len(), 1);
    assert_eq!(attribute(&events[0], "is_test_event"), Some(&json("true")));
    assert_eq!(
        attribute(&events[0], "app_name"),
        Some(&json(r#""my-app""#))
    );
    assert_eq!(
        attribute(&events[0], "source"),
        Some(&json(r#""litellm-proxy""#))
    );
    assert_eq!(
        number_value(&events[0].attributes, "timestamp").to_bits(),
        1_700_000_000.0_f64.to_bits()
    );
}

#[test]
fn a_disabled_integration_is_unhealthy_with_the_credentials_message() {
    let mut fixture = default_fixture();
    fixture.environment = environment(&[]);
    let logger = fixture.build();
    let status = async_health_check(&logger);
    assert_eq!(status.status, HealthState::Unhealthy);
    let message = status.error_message.expect("an unhealthy message");
    assert!(message.contains("NEW_RELIC_LICENSE_KEY"));
    assert!(message.contains("NEW_RELIC_APP_NAME"));
    assert!(message.contains("newrelic package is installed"));
}

#[test]
fn a_missing_agent_is_unhealthy_with_the_install_message() {
    let mut fixture = default_fixture();
    fixture.agent_kind = AgentKind::Missing;
    let logger = fixture.build();
    let status = async_health_check(&logger);
    assert_eq!(status.status, HealthState::Unhealthy);
    let message = status.error_message.expect("an unhealthy message");
    assert!(message.contains("newrelic package is installed"));
}

#[test]
fn an_application_that_is_not_enabled_is_unhealthy_with_the_agent_message() {
    let fixture = default_fixture();
    fixture.application.set_enabled(false);
    let logger = fixture.build();
    let status = async_health_check(&logger);
    assert_eq!(status.status, HealthState::Unhealthy);
    let message = status.error_message.expect("an unhealthy message");
    assert!(message.contains("New Relic Python agent not installed"));
    assert!(message.contains("docs.litellm.ai/docs/observability/newrelic"));
}

#[test]
fn a_failed_connection_test_is_unhealthy_with_the_agent_error() {
    let fixture = default_fixture();
    fixture.application.fail_event(CONNECTION_TEST_EVENT_NAME);
    let logger = fixture.build();
    let status = async_health_check(&logger);
    assert_eq!(status.status, HealthState::Unhealthy);
    let message = status.error_message.expect("an unhealthy message");
    assert!(message.contains("event boom"));
    assert!(fixture.counter.calls().is_empty());
}
