use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;

use crate::batch::logger::DEFAULT_MAX_QUEUE_SIZE;
use crate::callback_logger::CallbackLogger as _;
use crate::zerobus::client::init_params::ZerobusInitParams;
use crate::zerobus::logger::ZerobusLogger;
use crate::zerobus::logger::delivery::RowIngest;
use crate::zerobus::logger::params::configured_params;

use super::support::{
    FakeIngest, as_row_ingest, deps, event, expected_ids, logger, params, until_within,
};

#[test]
fn zerobus_defaults_match_litellm() {
    let defaults = ZerobusInitParams::default();
    assert_eq!(defaults.batch_size.get(), 100);
    assert_eq!(defaults.flush_interval.get(), 10);
    assert!(!defaults.turn_off_message_logging);
}

#[tokio::test]
async fn zerobus_defaults_build_the_queue_settings() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(100, 10, false), client);
    let settings = logger.inner.queue.settings;
    assert_eq!(settings.batch_size, 100);
    assert_eq!(settings.flush_interval, Duration::from_secs(10));
    assert_eq!(settings.max_queue_size, DEFAULT_MAX_QUEUE_SIZE);
    assert_eq!(settings.max_queue_size, 50_000);
}

#[tokio::test]
async fn zerobus_defaults_pass_the_configured_values() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut configured = deps(params(7, 3, true), Some(client));
    configured.max_queue_size = Some(12);
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    let settings = logger.inner.queue.settings;
    assert_eq!(settings.batch_size, 7);
    assert_eq!(settings.flush_interval, Duration::from_secs(3));
    assert_eq!(settings.max_queue_size, 12);
    assert!(logger.turn_off_message_logging());
}

#[test]
fn zerobus_defaults_convert_the_message_logging_flag() {
    let on = IndexMap::from([("turn_off_message_logging".to_owned(), "on".to_owned())]);
    let enabled = configured_params(Some(&on)).expect("the flag parses");
    assert!(enabled.turn_off_message_logging);
    let off = IndexMap::from([("turn_off_message_logging".to_owned(), "False".to_owned())]);
    let disabled = configured_params(Some(&off)).expect("the flag parses");
    assert!(!disabled.turn_off_message_logging);
}

#[tokio::test]
async fn zerobus_defaults_flush_on_the_interval() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let mut configured = deps(params(100, 1, false), Some(client));
    configured.start_periodic_flush = true;
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    logger
        .async_log_success_event(event("chatcmpl-t"))
        .await
        .expect("the event is accepted");
    assert!(
        until_within(10_000, || ingest.inserted() == 1).await,
        "the periodic flush runs"
    );
    assert_eq!(ingest.ids(), expected_ids(&["chatcmpl-t"]));
}
