use std::sync::Arc;
use std::time::Duration;

use crate::callback_logger::CallbackLogger;
use crate::datadog_logging::clock::{Clock as _, SystemClock};
use crate::datadog_logging::logger::uuid_id;
use crate::datadog_logging::settings::{DatadogInit, DatadogInitParams};
use crate::datadog_logging::tests::support;

#[tokio::test]
async fn callback_logger_reaches_the_async_events() {
    let (logger, _transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        Vec::new(),
    );
    let concrete_callback = Arc::clone(&logger);
    let callback: Arc<dyn CallbackLogger> = concrete_callback;
    callback
        .async_log_success_event(support::event(support::standard_payload("one", false)))
        .await
        .expect("the async success event is accepted");
    callback
        .async_log_failure_event(support::event(support::standard_payload("two", true)))
        .await
        .expect("the async failure event is accepted");
    assert_eq!(support::message_ids(&logger), vec!["one", "two"]);
}

#[test]
fn callback_logger_reports_its_name_and_options() {
    let (default_logger, _default_transport) = support::build(
        &support::agent_init(),
        &support::Environment::new(),
        Vec::new(),
    );
    let default_callback: &dyn CallbackLogger = default_logger.as_ref();
    assert_eq!(default_callback.callback_name(), "datadog");
    assert!(!default_callback.turn_off_message_logging());
    assert!(default_callback.message_logging());

    let quiet_init = DatadogInit {
        dd_api_key: None,
        dd_site: None,
        dd_agent_host: Some("agent.local".to_owned()),
        dd_agent_port: None,
        allow_env_credentials: true,
        params: DatadogInitParams {
            turn_off_message_logging: true,
        },
        use_v1: false,
    };
    let (quiet_logger, _quiet_transport) =
        support::build(&quiet_init, &support::Environment::new(), Vec::new());
    let quiet_callback: &dyn CallbackLogger = quiet_logger.as_ref();
    assert!(quiet_callback.turn_off_message_logging());
}

#[test]
fn callback_logger_reports_message_logging_from_its_options() {
    let (mut logger, _transport) = support::build(
        &support::agent_init(),
        &support::Environment::new(),
        Vec::new(),
    );
    Arc::get_mut(&mut logger)
        .expect("the logger has no other owner")
        .settings
        .options
        .message_logging = false;
    let callback: &dyn CallbackLogger = logger.as_ref();
    assert!(!callback.message_logging());
}

#[test]
fn uuid_id_is_a_hyphenated_version_4_uuid() {
    let id = uuid_id();
    let parsed = uuid::Uuid::parse_str(&id).expect("the id is a UUID");
    assert_eq!(id.len(), 36, "{id}");
    assert_eq!(id, parsed.hyphenated().to_string());
    assert_eq!(parsed.get_version_num(), 4, "{id}");
}

#[test]
fn uuid_id_returns_a_fresh_id_each_time() {
    assert_ne!(uuid_id(), uuid_id());
}

#[test]
fn the_system_clock_reports_the_time_since_the_epoch() {
    assert!(SystemClock.now_unix() > Duration::from_secs(1_700_000_000));
}
