use indexmap::IndexMap;

use crate::datadog::apm::NullTracer;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::proxy_failure::async_post_call_failure_hook;

use super::support::{
    Intake, build_logger, exception, exception_logging, field, json, last_queued, normalizer, text,
};

const TAGS: &str =
    "env:prod,service:litellm-server,version:unknown,HOSTNAME:host-1,POD_NAME:unknown";

#[tokio::test]
async fn datadog_proxy_queueing_flush_behavior() {
    let environment = [
        ("DD_BATCH_SIZE", "2"),
        ("DD_ENV", "prod"),
        ("HOSTNAME", "host-1"),
    ];
    let (logger, transport) = build_logger(&environment, Intake::Accepted);
    let patterns = normalizer();
    let facts =
        exception(r#"{"class_name": "APIConnectionError", "status_code": 500, "message": "down"}"#);
    let context = exception_logging(Some(&facts), &patterns);

    // the first failure stays queued, one short of the batch size
    async_post_call_failure_hook(&logger, &NullTracer, &context, None, IndexMap::new()).await;
    assert_eq!(logger.queue.pending().len(), 1);
    assert_eq!(transport.post_count(), 0);
    let queued = last_queued(&logger);
    assert_eq!(queued.status, DataDogStatus::Error);
    assert_eq!(queued.ddsource, "litellm");
    assert_eq!(queued.service, "litellm-server");
    assert_eq!(queued.hostname, "host-1");
    assert_eq!(queued.ddtags, TAGS);

    // the failure that reaches the batch size flushes everything queued
    async_post_call_failure_hook(&logger, &NullTracer, &context, None, IndexMap::new()).await;
    assert_eq!(logger.queue.pending().len(), 0);
    assert_eq!(transport.post_count(), 1);
    let delivered = transport.posted_events();
    assert_eq!(delivered.len(), 2);
    assert_eq!(text(field(&delivered[0], "status")), "error");
    assert_eq!(text(field(&delivered[1], "status")), "error");
    assert_eq!(text(field(&delivered[0], "ddtags")), TAGS);
    assert_eq!(text(field(&delivered[1], "ddtags")), TAGS);
    let first_inner = json(text(field(&delivered[0], "message")));
    assert_eq!(text(field(&first_inner, "exception")), "down");
    let second_inner = json(text(field(&delivered[1], "message")));
    assert_eq!(text(field(&second_inner, "exception")), "down");

    // after a flush the next failure starts a fresh batch and stays queued
    async_post_call_failure_hook(&logger, &NullTracer, &context, None, IndexMap::new()).await;
    assert_eq!(logger.queue.pending().len(), 1);
    assert_eq!(transport.post_count(), 1);

    // an unreachable intake returns the batch to the queue, and a queue past the batch size flushes again
    let (unreachable_logger, unreachable_transport) =
        build_logger(&[("DD_BATCH_SIZE", "2")], Intake::Unreachable);
    async_post_call_failure_hook(
        &unreachable_logger,
        &NullTracer,
        &context,
        None,
        IndexMap::new(),
    )
    .await;
    assert_eq!(unreachable_logger.queue.pending().len(), 1);
    async_post_call_failure_hook(
        &unreachable_logger,
        &NullTracer,
        &context,
        None,
        IndexMap::new(),
    )
    .await;
    assert_eq!(unreachable_transport.post_count(), 1);
    assert_eq!(unreachable_logger.queue.pending().len(), 2);
    async_post_call_failure_hook(
        &unreachable_logger,
        &NullTracer,
        &context,
        None,
        IndexMap::new(),
    )
    .await;
    assert_eq!(unreachable_transport.post_count(), 2);
    assert_eq!(unreachable_logger.queue.pending().len(), 3);
}
