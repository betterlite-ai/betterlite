use std::sync::Arc;

use crate::callback_logger::CallbackLogger;
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
