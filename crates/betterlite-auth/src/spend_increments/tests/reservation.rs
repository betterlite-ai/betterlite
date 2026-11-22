use crate::spend_increments::increment_spend_counters;

use super::support::{Fixture, Logs, applied_adjustment, decimal, finalized, handle, input};

#[tokio::test]
async fn a_reservation_adjustment_goes_out_first_in_the_one_apply() {
    let fixture = Fixture::new(true);
    fixture.layers.hold("spend:key:tok", decimal("5"));
    let reservation = handle("5", &["spend:key:tok"]);
    let mut request = input(Some("tok"));
    request.team_id = Some("t1");
    request.budget_reservation = Some(&reservation);

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    let pipelines = fixture.layers.pipelines();
    assert_eq!(pipelines.len(), 1);
    let applied = pipelines[0]
        .iter()
        .map(|item| (item.counter_key.clone(), item.increment))
        .collect::<Vec<_>>();
    assert_eq!(
        applied,
        vec![
            ("spend:key:tok".to_owned(), decimal("-3")),
            ("spend:team:t1".to_owned(), decimal("2")),
        ]
    );
    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:team:t1").await, Some(decimal("2")));
    assert_eq!(applied_adjustment(&reservation), Some(decimal("-3")));
    assert!(finalized(&reservation));
}

#[tokio::test]
async fn a_reconcile_failure_invalidates_and_falls_back_to_direct_increments() {
    let fixture = Fixture::new(false);
    let logs = Logs::capture();
    fixture.counters.fail_increment();
    let reservation = handle("5", &["spend:key:tok"]);
    let mut request = input(Some("tok"));
    request.budget_reservation = Some(&reservation);

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(
        fixture.counters.invalidated(),
        vec!["spend:key:tok".to_owned()]
    );
    assert_eq!(applied_adjustment(&reservation), Some(decimal("-3")));
    assert!(finalized(&reservation));
    assert!(logs.contains("Failed to reconcile budget reservation after persisted spend"));
}

#[tokio::test]
async fn an_unreadable_reserved_counter_is_reseeded_with_the_actual_cost() {
    let fixture = Fixture::new(false);
    let reservation = handle("5", &["spend:key:tok"]);
    let mut request = input(Some("tok"));
    request.budget_reservation = Some(&reservation);

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(
        fixture.counters.increments(),
        vec![("spend:key:tok".to_owned(), decimal("2"))]
    );
    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(applied_adjustment(&reservation), Some(decimal("-3")));
    assert!(finalized(&reservation));
}

#[tokio::test]
async fn a_missing_cost_settles_the_reservation_and_charges_no_scope() {
    let fixture = Fixture::new(true);
    fixture.layers.hold("spend:key:tok", decimal("5"));
    let reservation = handle("5", &["spend:key:tok"]);
    let mut request = input(Some("tok"));
    request.team_id = Some("t1");
    request.response_cost = None;
    request.budget_reservation = Some(&reservation);

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    let pipelines = fixture.layers.pipelines();
    assert_eq!(pipelines.len(), 1);
    assert_eq!(pipelines[0].len(), 1);
    assert_eq!(pipelines[0][0].counter_key, "spend:key:tok");
    assert_eq!(pipelines[0][0].increment, decimal("-5"));
    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("0")));
    assert_eq!(fixture.counter("spend:team:t1").await, None);
    assert_eq!(applied_adjustment(&reservation), Some(decimal("-5")));
    assert!(finalized(&reservation));
}

#[tokio::test]
async fn a_scope_error_still_stamps_the_reservation_but_never_finalizes_it() {
    let fixture = Fixture::new(false);
    fixture.batch.fail_for("spend:user:u1");
    let reservation = handle("5", &["spend:key:tok"]);
    let mut request = input(Some("tok"));
    request.user_id = Some("u1");
    request.budget_reservation = Some(&reservation);

    let result = increment_spend_counters(&request, &fixture.services()).await;

    assert!(result.is_err());
    assert_eq!(
        fixture.counters.increments(),
        vec![("spend:key:tok".to_owned(), decimal("2"))]
    );
    assert_eq!(applied_adjustment(&reservation), Some(decimal("-3")));
    assert!(!finalized(&reservation));
}
