use std::sync::Arc;

use betterlite_models::decisions::Json;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog_llm_observability::tests::support;
use crate::datadog_logging::logger::uuid_id;

fn logger() -> (Arc<support::Logger>, Arc<support::ScriptedTransport>) {
    support::build(
        &support::agent_init(),
        &support::environment(&[]),
        Vec::new(),
    )
}

#[tokio::test]
async fn callback_logger_reports_the_llm_observability_name_and_flags() {
    let (logger, _transport) = logger();
    assert_eq!(logger.callback_name(), "datadog_llm_observability");
    assert!(!logger.turn_off_message_logging());
    assert!(logger.message_logging());
    assert!(logger.redacts_messages_itself());
}

#[tokio::test]
async fn callback_logger_queues_the_success_event() {
    let (logger, _transport) = logger();
    logger
        .async_log_success_event(support::event(support::standard_payload("ok")))
        .await
        .expect("the success event is accepted");
    assert_eq!(logger.queue.pending().len(), 1);
}

#[tokio::test]
async fn callback_logger_queues_the_failure_event() {
    let (logger, _transport) = logger();
    logger
        .async_log_failure_event(support::event(support::failure_payload("failed")))
        .await
        .expect("the failure event is accepted");
    assert_eq!(logger.queue.pending().len(), 1);
}

#[tokio::test]
async fn callback_logger_reports_the_configured_redaction_flags() {
    let mut init = support::agent_init();
    init.params = Some(support::params(Some(true)));
    init.message_logging = Some(false);
    let (logger, _transport) = support::build(&init, &support::environment(&[]), Vec::new());
    assert!(logger.turn_off_message_logging());
    assert!(!logger.message_logging());
}

#[tokio::test]
async fn callback_logger_accepts_dynamic_parameters_that_are_not_an_object() {
    let (logger, _transport) = logger();
    logger
        .async_log_success_event(support::event_with(
            support::standard_payload("dynamic"),
            Json::Text("not an object".to_owned()),
        ))
        .await
        .expect("the event is accepted");
    assert_eq!(logger.queue.pending().len(), 1);
}

#[test]
fn uuid_id_generates_a_version_four_uuid() {
    let first = uuid_id();
    let second = uuid_id();
    assert_eq!(first.len(), 36);
    uuid::Uuid::parse_str(&first).expect("the id is a uuid");
    assert_ne!(first, second);
}
