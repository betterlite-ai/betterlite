#![cfg(test)]

mod async_handler;
mod dispatch;
mod handler;
mod helper_fn;
mod partial_usage;
mod support;

use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::logging_failure::FailureRecord;
use crate::logging_failure::handlers::dispatch_failure_handlers;
use crate::logging_state::correlation::CorrelationContext;

use self::support::{
    CallMethod, FakeLogger, Harness, build, call, callbacks, init, object, params,
    registry_with_channel, report, text,
};

// prefer_async runs the async delivery and then the queued sync job; otherwise the sync handler runs alone
#[rstest]
#[case::async_then_queued_sync(true, &[CallMethod::Async, CallMethod::Sync])]
#[case::sync_handler(false, &[CallMethod::Sync])]
#[tokio::test]
async fn failure_events_carry_litellm_params_metadata(
    #[case] prefer_async_handlers: bool,
    #[case] methods: &[CallMethod],
) {
    let logger = Arc::new(FakeLogger::new("langsmith"));
    let registry = registry_with_channel(&[Arc::clone(&logger)]);
    let harness = Harness::new();
    let mut logging_init = init();
    logging_init.litellm_params = params(&[("metadata", object(&[("run_name", text("my-run"))]))]);
    let mut state = build(logging_init);
    let mut record = FailureRecord::default();
    let mut context = CorrelationContext::default();
    dispatch_failure_handlers(
        &mut call(&mut state, &mut record, &mut context),
        &report("provider exploded"),
        &callbacks(&registry),
        &harness.deps(),
        prefer_async_handlers,
    )
    .await;
    let mut wanted = IndexMap::new();
    wanted.insert("run_name".to_owned(), Json::Text("my-run".to_owned()));
    let expected: Vec<(CallMethod, Arc<IndexMap<String, Json>>)> = methods
        .iter()
        .map(|method| (*method, Arc::new(wanted.clone())))
        .collect();
    assert_eq!(logger.litellm_metadata(), expected);
}
