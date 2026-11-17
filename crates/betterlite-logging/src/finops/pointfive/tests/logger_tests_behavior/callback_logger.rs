use std::sync::Arc;

use betterlite_config::sections::pointfive_params::PointFiveInitParams;

use crate::callback_logger::CallbackLogger;
use crate::finops::pointfive::logger::PointFiveLogger;

use super::support::{FakeHttp, client, default_params, event, logger, payload};

#[tokio::test]
async fn callback_logger_reports_its_name() {
    let http = FakeHttp::default();
    let logger: Arc<dyn CallbackLogger> = logger(PointFiveInitParams::default(), client(&http, 1));
    assert_eq!(logger.callback_name(), "pointfive");
}

#[test]
fn callback_logger_reports_the_configured_redaction_flag() {
    let http = FakeHttp::default();
    let off: Arc<dyn CallbackLogger> = logger(default_params(), client(&http, 1));
    assert!(!off.turn_off_message_logging());
    let mut params = default_params();
    params.turn_off_message_logging = true;
    let on: Arc<dyn CallbackLogger> = logger(params, client(&http, 1));
    assert!(on.turn_off_message_logging());
    assert!(!on.redacts_messages_itself());
}

#[tokio::test]
async fn callback_logger_buffers_a_success_event() {
    let http = FakeHttp::default();
    let logger = logger(PointFiveInitParams::default(), client(&http, 1));
    let trait_logger: Arc<dyn CallbackLogger> = Arc::<PointFiveLogger>::clone(&logger);
    trait_logger
        .async_log_success_event(event(payload("evt-1")))
        .await
        .expect("a buffered event never fails the request");
    assert!(
        logger
            .core
            .queue
            .snapshot()
            .first()
            .is_some_and(|record| record.id == "evt-1")
    );
}

#[tokio::test]
async fn callback_logger_buffers_a_failure_event() {
    let http = FakeHttp::default();
    let logger = logger(PointFiveInitParams::default(), client(&http, 1));
    let trait_logger: Arc<dyn CallbackLogger> = Arc::<PointFiveLogger>::clone(&logger);
    trait_logger
        .async_log_failure_event(event(payload("evt-2")))
        .await
        .expect("a buffered event never fails the request");
    assert!(
        logger
            .core
            .queue
            .snapshot()
            .first()
            .is_some_and(|record| record.id == "evt-2")
    );
}
