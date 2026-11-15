use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::advance;

use crate::langfuse_channels::{CHANNEL_RETIRE_GRACE, release_langfuse_tracing};
use crate::langfuse_logger::LangfuseLoggerError;
use crate::langfuse_logger::langfuse::{LangFuseLogger, LangfuseLoggedEvent};

use super::support;

#[tokio::test]
async fn flush_drains_the_channel() {
    let services = support::services(&[], 10);
    let request = support::request("pk-flush", "sk-flush");
    let (exporter, recording) = support::recording();
    let _probe = support::seed(&services, &request, exporter);
    let logger = LangFuseLogger::new(&services, &request)
        .await
        .expect("the logger builds");
    let payload = support::payload(support::TRACE_ID);
    let response = support::response();
    let params = support::optional_params();
    let litellm_params = support::litellm_params(false);
    let call = support::call(
        &payload,
        &response,
        &params,
        &litellm_params,
        support::start(),
    );
    let event = logger.log_event_on_langfuse(&call).await;
    assert_eq!(event.trace_id.as_deref(), Some(support::TRACE_ID));
    assert!(recording.spans().is_empty());
    assert!(logger.flush().await);
    assert!(!recording.spans().is_empty());
}

#[tokio::test]
async fn a_failed_export_makes_the_flush_report_failure() {
    let services = support::services(&[], 10);
    let request = support::request("pk-flush-fail", "sk-flush-fail");
    let _probe = support::seed(&services, &request, support::Rejecting);
    let logger = LangFuseLogger::new(&services, &request)
        .await
        .expect("the logger builds");
    let payload = support::payload(support::TRACE_ID);
    let response = support::response();
    let params = support::optional_params();
    let litellm_params = support::litellm_params(false);
    let call = support::call(
        &payload,
        &response,
        &params,
        &litellm_params,
        support::start(),
    );
    let (log, guard) = support::capture_log();
    let _event = logger.log_event_on_langfuse(&call).await;
    assert!(!logger.flush().await);
    assert!(
        log.contents()
            .contains("the langfuse export channel did not drain before the flush deadline")
    );
    drop(guard);
}

#[tokio::test(start_paused = true)]
async fn stop_keeps_the_channel_for_the_grace_period() {
    let services = support::services(&[], 10);
    let request = support::request("pk-grace", "sk-grace");
    let (exporter, _recording) = support::recording();
    let probe = support::seed(&services, &request, exporter);
    let logger = LangFuseLogger::new(&services, &request)
        .await
        .expect("the logger builds");
    release_langfuse_tracing(&probe, CHANNEL_RETIRE_GRACE).await;
    logger.stop();
    yield_now().await;
    let (rebuilt_exporter, rebuilt) = support::recording();
    let _reseeded = support::seed(&services, &request, rebuilt_exporter);
    assert_eq!(rebuilt.installed(), 0);
}

#[tokio::test(start_paused = true)]
async fn stop_retires_the_channel_after_the_grace_period() {
    let services = support::services(&[], 10);
    let request = support::request("pk-retire", "sk-retire");
    let (exporter, _recording) = support::recording();
    let probe = support::seed(&services, &request, exporter);
    let logger = LangFuseLogger::new(&services, &request)
        .await
        .expect("the logger builds");
    release_langfuse_tracing(&probe, CHANNEL_RETIRE_GRACE).await;
    logger.stop();
    yield_now().await;
    yield_now().await;
    advance(Duration::from_secs(60)).await;
    yield_now().await;
    let (rebuilt_exporter, rebuilt) = support::recording();
    let _reseeded = support::seed(&services, &request, rebuilt_exporter);
    assert_eq!(rebuilt.installed(), 1);
}

#[tokio::test]
async fn stop_is_idempotent() {
    let services = support::services(&[("LANGFUSE_MOCK", "true")], 2);
    let first = LangFuseLogger::new(&services, &support::request("pk-idem-1", "sk-idem-1"))
        .await
        .expect("the first logger builds");
    let _second = LangFuseLogger::new(&services, &support::request("pk-idem-2", "sk-idem-2"))
        .await
        .expect("the second logger builds");
    first.stop();
    first.stop();
    let _third = LangFuseLogger::new(&services, &support::request("pk-idem-3", "sk-idem-3"))
        .await
        .expect("the released slot is free");
    let error = LangFuseLogger::new(&services, &support::request("pk-idem-4", "sk-idem-4"))
        .await
        .expect_err("the budget stays full");
    assert!(matches!(error, LangfuseLoggerError::Budget(_)));
}

#[tokio::test]
async fn flush_and_stop_are_callable_after_a_failed_event() {
    let services = support::services(&[], 10);
    let request = support::request("pk-life-fail", "sk-life-fail");
    let (exporter, _recording) = support::recording();
    let _probe = support::seed(&services, &request, exporter);
    let logger = LangFuseLogger::new(&services, &request)
        .await
        .expect("the logger builds");
    let payload = support::payload(support::TRACE_ID);
    let response = support::failed_response();
    let params = support::optional_params();
    let litellm_params = support::litellm_params(false);
    let call = support::call(
        &payload,
        &response,
        &params,
        &litellm_params,
        support::start(),
    );
    let event = logger.log_event_on_langfuse(&call).await;
    assert_eq!(event, LangfuseLoggedEvent::default());
    assert!(logger.flush().await);
    logger.stop();
}
