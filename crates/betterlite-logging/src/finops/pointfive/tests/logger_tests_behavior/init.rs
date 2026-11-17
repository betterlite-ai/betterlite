use std::sync::Arc;

use betterlite_config::sections::pointfive_params::PointFiveInitParams;

use crate::callback_logger::CallbackLogger as _;
use crate::finops::pointfive::logger::PointFiveLogger;
use crate::finops::pointfive::logger::error::PointFiveError;
use crate::finops::pointfive::upload::PointFiveUploadClient;

use super::support::{
    FakeHttp, FixedSource, MissingKeySource, Reply, client, default_params, deps_from, event,
    payload,
};

fn fixed(upload_client: PointFiveUploadClient) -> Arc<FixedSource> {
    Arc::new(FixedSource {
        client: upload_client,
        calls: Arc::default(),
    })
}

#[test]
fn the_logger_keeps_the_configured_params() {
    let mut params = default_params();
    params.batch_size = 7;
    params.flush_interval = 9;
    params.max_batch_bytes = 4242;
    params.max_upload_retries = 2;
    params.turn_off_message_logging = true;
    let http = FakeHttp::default();
    let upload_client = client(&http, 2);
    let logger = PointFiveLogger::new(
        params,
        deps_from(fixed(upload_client.clone()), Some(upload_client), false),
    )
    .expect("the logger builds");
    assert_eq!(logger.core.params.batch_size, 7);
    assert_eq!(logger.core.params.flush_interval, 9);
    assert_eq!(logger.core.params.max_batch_bytes, 4242);
    assert_eq!(logger.core.params.max_upload_retries, 2);
    assert!(logger.turn_off_message_logging());
}

#[test]
fn a_missing_key_refuses_to_build_the_logger() {
    let error = PointFiveLogger::new(
        PointFiveInitParams::default(),
        deps_from(Arc::new(MissingKeySource), None, false),
    )
    .err()
    .expect("the key is missing");
    assert!(matches!(error, PointFiveError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "pointfive logging requires an api key. Set POINTFIVE_API_KEY, or litellm_settings.pointfive_params.api_key in config.yaml"
    );
}

#[test]
fn a_given_client_skips_the_key_check() {
    let http = FakeHttp::default();
    let logger = PointFiveLogger::new(
        PointFiveInitParams::default(),
        deps_from(Arc::new(MissingKeySource), Some(client(&http, 1)), false),
    )
    .expect("the given client needs no key");
    assert!(logger.core.given_upload_client.is_some());
}

#[test]
fn construction_outside_a_runtime_leaves_the_flush_unscheduled() {
    let http = FakeHttp::default();
    let upload_client = client(&http, 1);
    let logger = PointFiveLogger::new(
        PointFiveInitParams::default(),
        deps_from(fixed(upload_client.clone()), Some(upload_client), true),
    )
    .expect("the logger builds");
    assert!(!logger.flush_task_is_alive());
    assert!(
        logger
            .periodic_task
            .lock()
            .expect("the lock is not poisoned")
            .is_none()
    );
}

#[tokio::test]
async fn construction_inside_a_runtime_starts_the_periodic_flush() {
    let http = FakeHttp::scripted(vec![Reply::answer(200)]);
    let upload_client = client(&http, 1);
    let logger = PointFiveLogger::new(
        PointFiveInitParams::default(),
        deps_from(fixed(upload_client.clone()), Some(upload_client), true),
    )
    .expect("the logger builds");
    assert!(logger.flush_task_is_alive());
    logger.shutdown.cancel();
    let task = logger
        .periodic_task
        .lock()
        .expect("the lock is not poisoned")
        .take()
        .expect("the periodic task was started");
    task.await.expect("the periodic task ends");
    assert_eq!(http.pings().len(), 1);
}

#[tokio::test]
async fn without_the_flag_the_periodic_flush_waits_for_the_first_event() {
    let http = FakeHttp::default();
    let upload_client = client(&http, 1);
    let logger = PointFiveLogger::new(
        PointFiveInitParams::default(),
        deps_from(fixed(upload_client.clone()), Some(upload_client), false),
    )
    .expect("the logger builds");
    assert!(!logger.flush_task_is_alive());
    logger
        .async_log_success_event(event(payload("evt-1")))
        .await
        .expect("the event is queued");
    assert!(logger.flush_task_is_alive());
    logger.shutdown.cancel();
}
