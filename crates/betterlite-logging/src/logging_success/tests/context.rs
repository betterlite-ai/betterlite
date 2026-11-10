use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{CallLog, FakeLogger, Fixture, chat_result, correlated_state, request};
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_success::dispatch::dispatch_success_handlers;

#[rstest]
#[case::after_success(false, false)]
#[case::after_error(false, true)]
#[case::async_after_error(true, true)]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(#[case] asynchronous: bool, #[case] fail: bool) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    let mut logger = FakeLogger::new("langfuse", &calls);
    logger.fail = fail;
    fixture.add(&Arc::new(logger));
    let mut context = CorrelationContext {
        trace_id: "outer-trace".to_owned(),
        session_id: "outer-session".to_owned(),
    };
    let mut state = correlated_state(Some(false), &mut context);
    assert_eq!(context.trace_id, "inner-trace");
    if asynchronous {
        state
            .litellm_params
            .insert("acompletion".to_owned(), Json::Bool(true));
    }
    dispatch_success_handlers(
        &fixture.handler(),
        &mut state,
        &mut context,
        &request(chat_result()),
    )
    .await;
    assert_eq!(context.trace_id, "outer-trace");
    assert_eq!(context.session_id, "outer-session");
    assert_eq!(calls.events().len(), 1);
}
