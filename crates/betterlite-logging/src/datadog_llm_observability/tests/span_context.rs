use std::sync::Arc;

use rstest::rstest;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog::apm::{Span, Tracer};
use crate::datadog_llm_observability::tests::support::{self, TestSpan, TestTracer};

struct RootSpanTracer(TestSpan);

impl Tracer for RootSpanTracer {
    fn current_root_span(&self) -> Option<&dyn Span> {
        Some(&self.0)
    }
}

#[rstest]
#[case::current_span_trace_id(
    Arc::new(TestTracer { span: Some(TestSpan { trace_id: Some(12) }) }),
    Some("12")
)]
#[case::largest_trace_id(
    Arc::new(TestTracer { span: Some(TestSpan { trace_id: Some(u128::MAX) }) }),
    Some("340282366920938463463374607431768211455")
)]
#[case::span_without_trace_id(
    Arc::new(TestTracer { span: Some(TestSpan { trace_id: None }) }),
    None
)]
#[case::no_current_span(Arc::new(TestTracer { span: None }), None)]
#[case::root_span_is_not_read(Arc::new(RootSpanTracer(TestSpan { trace_id: Some(7) })), None)]
#[case::no_tracer(support::null_tracer(), None)]
#[tokio::test]
async fn datadog_llm_obs_span_context_attachment_behavior(
    #[case] tracer: Arc<dyn Tracer + Send + Sync>,
    #[case] expected: Option<&str>,
) {
    let environment = support::environment(&[]);
    let (logger, _transport) = support::build_with(
        &support::agent_init(),
        &environment,
        Vec::new(),
        tracer,
        None,
    );
    logger
        .async_log_success_event(support::event(support::standard_payload("one")))
        .await
        .expect("the event is accepted");
    let queued = logger.queue.pending();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].apm_id.as_deref(), expected);
}
