use std::collections::BTreeMap;
use std::time::Duration;

use crate::prometheus_remaining_budget::fan_out::RemainingBudgetFanOut;

use super::support::{
    Behaviour, RealGauge, Recorder, capture_log, collectors_with, logged_text, metadata, request,
};

#[tokio::test]
async fn two_failing_lookups_log_and_leave_the_other_three_done() {
    let recorder = Recorder::new(BTreeMap::from([
        ("key", Behaviour::Failing),
        ("org", Behaviour::Failing),
    ]));
    let fan_out = RemainingBudgetFanOut::new(
        recorder.clone(),
        Duration::from_secs(5),
        &collectors_with(&[RealGauge::Team]),
    );
    let (guard, log) = capture_log();
    fan_out
        .increment_remaining_budget_metrics(request(&metadata(&[])))
        .await;
    drop(guard);
    assert_eq!(recorder.calls().len(), 5);
    assert_eq!(recorder.finished(), ["team", "user", "customer"]);
    let text = logged_text(&log);
    assert!(
        text.contains("Budget metric lookup key failed: key lookup failed"),
        "{text}"
    );
    assert!(
        text.contains("Budget metric lookup org failed: org lookup failed"),
        "{text}"
    );
    assert!(!text.contains("Budget metric lookup team failed"), "{text}");
    assert!(!text.contains("Budget metric lookup user failed"), "{text}");
    assert!(
        !text.contains("Budget metric lookup customer failed"),
        "{text}"
    );
    assert!(!text.contains("exceeded"), "{text}");
}
