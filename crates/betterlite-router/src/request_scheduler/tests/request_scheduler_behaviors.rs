use std::time::Duration;

use betterlite_config::sections::request_scheduler::Settings;
use rstest::rstest;

use crate::request_scheduler::error::SchedulerError;
use crate::request_scheduler::flow::Priority;
use crate::request_scheduler::schedule::PRIORITIZATION_HEADER;
use crate::routing_state_store::error::StoreWarning;

use super::{MODEL, ScriptedCall, entries, item, plan, scheduler, shared_scheduler};

// Scheduler.add_request + get_queue: the queue serves lower priorities first and the request id breaks a tie
#[rstest]
#[case::lower_number_runs_first(vec![(255, "low"), (0, "high"), (128, "medium")], vec![(0, "high"), (128, "medium"), (255, "low")])]
#[case::equal_priorities_order_by_request_id(vec![(128, "b"), (128, "a")], vec![(128, "a"), (128, "b")])]
#[case::a_three_digit_priority_sorts_after_two_digits(vec![(255, "z"), (20, "y"), (3, "x")], vec![(3, "x"), (20, "y"), (255, "z")])]
#[tokio::test]
async fn queued_requests_run_in_priority_order(
    #[case] added: Vec<(u8, &'static str)>,
    #[case] expected: Vec<(u8, &'static str)>,
) {
    let service = scheduler(Settings::default());
    for (priority, request_id) in added {
        service
            .add_request(&item(priority, request_id, MODEL))
            .await
            .expect("the request queues");
    }
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&expected)
    );
}

// Scheduler.poll returns True without popping when a healthy deployment exists, whoever is at the head
#[tokio::test]
async fn a_healthy_deployment_serves_a_request_behind_the_head() {
    let service = scheduler(Settings::default());
    service
        .add_request(&item(Priority::High.value(), "head", MODEL))
        .await
        .expect("the head queues");
    service
        .add_request(&item(Priority::Low.value(), "late", MODEL))
        .await
        .expect("the late request queues");
    let allowed = service
        .poll("late", MODEL, 1)
        .await
        .expect("the poll answers");
    assert!(allowed.value);
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(0, "head"), (255, "late")])
    );
}

// without a healthy deployment only `queue[0]` runs, and a poll of it takes the entry out
#[tokio::test]
async fn without_a_healthy_deployment_only_the_head_runs() {
    let service = scheduler(Settings::default());
    service
        .add_request(&item(Priority::Medium.value(), "head", MODEL))
        .await
        .expect("the head queues");
    service
        .add_request(&item(Priority::Low.value(), "late", MODEL))
        .await
        .expect("the late request queues");
    let behind = service
        .poll("late", MODEL, 0)
        .await
        .expect("the poll answers");
    assert!(!behind.value);
    let head = service
        .poll("head", MODEL, 0)
        .await
        .expect("the poll answers");
    assert!(head.value);
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(255, "late")])
    );
}

// Scheduler.peek compares the head and leaves the heap as it was
#[tokio::test]
async fn peek_reports_the_head_without_removing_it() {
    let service = scheduler(Settings::default());
    for (priority, request_id) in [(128, "b"), (128, "a")] {
        service
            .add_request(&item(priority, request_id, MODEL))
            .await
            .expect("the request queues");
    }
    let head = service.peek("a", MODEL).await.expect("the peek answers");
    assert!(head.value);
    let behind = service.peek("b", MODEL).await.expect("the peek answers");
    assert!(!behind.value);
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(128, "a"), (128, "b")])
    );
}

// get_queue is keyed by model group: one group's entries never reach another's queue
#[tokio::test]
async fn each_model_has_its_own_queue() {
    let service = scheduler(Settings::default());
    service
        .add_request(&item(0, "first", "gpt-4o"))
        .await
        .expect("the request queues");
    service
        .add_request(&item(255, "second", "gpt-4o-mini"))
        .await
        .expect("the request queues");
    assert_eq!(
        service
            .queue("gpt-4o")
            .await
            .expect("the queue reads")
            .value,
        entries(&[(0, "first")])
    );
    assert_eq!(
        service
            .queue("gpt-4o-mini")
            .await
            .expect("the queue reads")
            .value,
        entries(&[(255, "second")])
    );
}

// Router.schedule_acompletion: the response reports the prioritization header and the entry leaves the queue
#[tokio::test(start_paused = true)]
async fn a_scheduled_request_reports_the_prioritization_header() {
    let service = scheduler(Settings::default());
    let call = ScriptedCall::answered(0);
    let outcome = service
        .schedule(&item(7, "scheduled", MODEL), &call, &plan(0.5))
        .await
        .expect("the request is scheduled");
    assert!(outcome.prioritized);
    assert_eq!(outcome.priority, 7);
    assert_eq!(outcome.polls, 0);
    assert_eq!(outcome.request_id, "scheduled");
    assert_eq!(
        PRIORITIZATION_HEADER,
        "x-litellm-request-prioritization-used"
    );
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[])
    );
}

// a healthy deployment lets a queued request run while the head waits, and only that request leaves the queue
#[tokio::test(start_paused = true)]
async fn a_healthy_deployment_bypasses_the_queue_head() {
    let service = scheduler(Settings::default());
    service
        .add_request(&item(Priority::High.value(), "head", MODEL))
        .await
        .expect("the head queues");
    let call = ScriptedCall::answered(2);
    let outcome = service
        .schedule(
            &item(Priority::Low.value(), "scheduled", MODEL),
            &call,
            &plan(0.5),
        )
        .await
        .expect("the request is scheduled");
    assert!(outcome.prioritized);
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(0, "head")])
    );
}

// Router.acompletion's `kwargs.get("priority") or self.default_priority`, with no configured default by default
#[tokio::test]
async fn the_configured_default_priority_fills_a_request_without_one() {
    let configured = scheduler(Settings {
        default_priority: Some(Priority::Medium.value()),
        ..Settings::default()
    });
    assert_eq!(configured.priority(None), Some(128));
    assert_eq!(configured.priority(Some(Priority::High.value())), Some(0));
    let none = scheduler(Settings::default());
    assert_eq!(none.priority(None), None);
}

// Router.schedule_acompletion's str(uuid.uuid4()) comes from the injected source
#[test]
fn the_request_id_source_names_each_scheduled_request() {
    assert_eq!(
        scheduler(Settings::default()).next_request_id(),
        "generated"
    );
}

// polling_interval or DEFAULT_POLLING_INTERVAL, 0.03 seconds at the pin
#[tokio::test]
async fn polling_interval_defaults_to_thirty_milliseconds() {
    let defaults = scheduler(Settings::default());
    assert_eq!(defaults.polling_interval(), Duration::from_millis(30));
    let configured = scheduler(Settings {
        polling_interval: 0.25_f64,
        ..Settings::default()
    });
    assert_eq!(configured.polling_interval(), Duration::from_millis(250));
}

// DualCache reads Redis first and keeps serving from memory when the shared tier stops answering
#[tokio::test]
async fn a_shared_queue_survives_an_unavailable_tier() {
    let (service, server) = shared_scheduler().await;
    service
        .add_request(&item(128, "first", MODEL))
        .await
        .expect("the request queues");
    assert_eq!(
        service.queue(MODEL).await.expect("the queue reads").value,
        entries(&[(128, "first")])
    );
    server.set_down(true);
    let added = service
        .add_request(&item(0, "second", MODEL))
        .await
        .expect("the fallback queue accepts the request");
    assert!(
        matches!(added.warning, Some(StoreWarning::Unavailable { .. })),
        "the fallback reports its warning"
    );
    let queued = service.queue(MODEL).await.expect("the queue reads");
    assert!(
        matches!(queued.warning, Some(StoreWarning::Unavailable { .. })),
        "the read reports its warning"
    );
    assert_eq!(queued.value, entries(&[(0, "second"), (128, "first")]));
}

// an empty queue is the setup error Scheduler.poll and Scheduler.peek raise, told apart from the wait
#[tokio::test]
async fn an_empty_queue_is_refused() {
    let service = scheduler(Settings::default());
    let polled = service.poll("missing", MODEL, 0).await;
    assert!(matches!(polled, Err(SchedulerError::EmptyQueue { .. })));
    let peeked = service.peek("missing", MODEL).await;
    assert!(matches!(peeked, Err(SchedulerError::EmptyQueue { .. })));
}
