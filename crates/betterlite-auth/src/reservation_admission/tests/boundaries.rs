use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::reservation_admission::engine::{AdmissionRequest, admit};
use crate::reservation_admission::error::AdmissionError;
use crate::reservation_admission::reserve::{GroupCharge, settle_group};
use crate::reservation_settlement::entry::{ReservationEntry, counter_to_reservation_entry};
use crate::reservation_settlement::release::release_entries_best_effort;

use super::support::{
    KEY, USER, WINDOW, amount, entity, request, seeded, take_calls, value_of, window,
};

const TEAM: &str = "spend:team:t1";

fn entry_keys(applied: &[ReservationEntry]) -> Vec<String> {
    applied
        .iter()
        .filter_map(|entry| entry.counter_key.clone())
        .collect()
}

// a cold window has no row to load, so it is skipped without being invalidated
async fn a_cold_window_is_skipped_without_invalidating() {
    let counters = vec![
        entity(KEY, "10", "0", CounterEntityType::Key),
        window(WINDOW, "5"),
        entity(USER, "20", "0", CounterEntityType::User),
    ];
    let fake = seeded(&[(KEY, "0"), (USER, "0")], &[]);
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect("the admission");
    assert_eq!(admission.reserved_cost, amount("4"));
    assert_eq!(entry_keys(&applied), vec![KEY, USER]);
    let calls = take_calls(&fake);
    assert_eq!(calls.windows, vec![WINDOW.to_owned()]);
    assert!(calls.invalidations.is_empty());
    assert_eq!(value_of(&fake, WINDOW), None);
}

// a window whose seed failed is invalidated and skipped, and strict enforcement rejects the request
async fn a_failed_window_seed_invalidates_and_rejects() {
    let counters = vec![window(WINDOW, "5")];
    let mut fake = seeded(&[], &[]);
    let _inserted = fake.failed_seeds.insert(WINDOW.to_owned());
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect("the admission");
    assert_eq!(admission.reserved_cost, amount("4"));
    assert!(applied.is_empty());
    assert_eq!(take_calls(&fake).invalidations, vec![WINDOW.to_owned()]);
    let mut strict = seeded(&[], &[]);
    let _inserted = strict.failed_seeds.insert(WINDOW.to_owned());
    let strict_request = AdmissionRequest {
        fail_closed_budget_enforcement: true,
        ..request(&counters, "4")
    };
    let error = admit(&strict, &strict_request, &mut Vec::new())
        .await
        .expect_err("the rejection");
    let AdmissionError::Unavailable { counter_key } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(counter_key, WINDOW);
    assert_eq!(take_calls(&strict).invalidations, vec![WINDOW.to_owned()]);
}

// one counter over budget stops the groups after it, and the caller releases exactly what was reserved
async fn a_rejection_stops_before_later_groups() {
    let counters = vec![
        entity(KEY, "10", "0", CounterEntityType::Key),
        entity(USER, "3", "3", CounterEntityType::User),
        entity(TEAM, "10", "0", CounterEntityType::Team),
    ];
    let fake = seeded(&[(KEY, "0"), (USER, "3"), (TEAM, "0")], &[]);
    let mut applied = Vec::new();
    let error = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect_err("the block");
    let AdmissionError::BudgetExceeded { details } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(details.current_cost, amount("7"));
    assert_eq!(details.max_budget, amount("3"));
    assert_eq!(entry_keys(&applied), vec![KEY, USER]);
    assert_eq!(value_of(&fake, TEAM), Some(Decimal::ZERO));
    let calls = take_calls(&fake);
    assert_eq!(
        calls.reservations,
        vec![
            vec![(KEY.to_owned(), amount("4"))],
            vec![(USER.to_owned(), amount("4"))],
        ]
    );
    release_entries_best_effort(&fake, &mut applied, amount("4")).await;
    assert_eq!(value_of(&fake, KEY), Some(Decimal::ZERO));
    assert_eq!(value_of(&fake, USER), Some(amount("3")));
    assert_eq!(value_of(&fake, TEAM), Some(Decimal::ZERO));
}

// a pipeline that fails drops its own group and leaves the groups before and after it alone
async fn a_failed_pipeline_rolls_back_only_its_group() {
    let counters = vec![
        entity(KEY, "10", "0", CounterEntityType::Key),
        entity(USER, "5", "5", CounterEntityType::User),
        entity(TEAM, "10", "0", CounterEntityType::Team),
    ];
    let mut fake = seeded(&[(KEY, "0"), (USER, "5"), (TEAM, "0")], &[]);
    let _inserted = fake.failed_pipelines.insert(USER.to_owned());
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect("the admission");
    assert_eq!(admission.reserved_cost, amount("4"));
    assert_eq!(entry_keys(&applied), vec![KEY, TEAM]);
    assert_eq!(value_of(&fake, KEY), Some(amount("4")));
    assert_eq!(value_of(&fake, USER), None);
    assert_eq!(value_of(&fake, TEAM), Some(amount("4")));
    assert_eq!(take_calls(&fake).invalidations, vec![USER.to_owned()]);
    let mut strict = seeded(&[(KEY, "0"), (USER, "5"), (TEAM, "0")], &[]);
    let _inserted = strict.failed_pipelines.insert(USER.to_owned());
    let mut rejected = Vec::new();
    let strict_request = AdmissionRequest {
        fail_closed_budget_enforcement: true,
        ..request(&counters, "4")
    };
    let error = admit(&strict, &strict_request, &mut rejected)
        .await
        .expect_err("the rejection");
    let AdmissionError::Unavailable { counter_key } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(counter_key, USER);
    assert_eq!(error.status_code(), Some(503));
    assert_eq!(entry_keys(&rejected), vec![KEY]);
    assert_eq!(value_of(&strict, TEAM), Some(Decimal::ZERO));
}

// a remaining budget of exactly 1e-12 blocks, and twice that resizes
async fn the_remaining_budget_tolerance_decides() {
    let blocked = vec![entity(
        USER,
        "10",
        "9.999999999999",
        CounterEntityType::User,
    )];
    let fake = seeded(&[(USER, "9.999999999999")], &[]);
    let error = admit(&fake, &request(&blocked, "4"), &mut Vec::new())
        .await
        .expect_err("the block");
    let AdmissionError::BudgetExceeded { details } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(details.current_cost, amount("13.999999999999"));
    let resized = vec![entity(
        USER,
        "10",
        "9.999999999998",
        CounterEntityType::User,
    )];
    let fake = seeded(&[(USER, "9.999999999998")], &[]);
    let admission = admit(&fake, &request(&resized, "4"), &mut Vec::new())
        .await
        .expect("the resize");
    assert_eq!(admission.reserved_cost, amount("0.000000000002"));
}

// strict enforcement rejects only an overshoot beyond 1e-12
async fn the_strict_overshoot_tolerance_decides() {
    let inside = vec![entity(
        USER,
        "10",
        "6.000000000001",
        CounterEntityType::User,
    )];
    let fake = seeded(&[(USER, "6.000000000001")], &[]);
    let strict_request = AdmissionRequest {
        fail_closed_budget_enforcement: true,
        ..request(&inside, "4")
    };
    let admission = admit(&fake, &strict_request, &mut Vec::new())
        .await
        .expect("the resize at the tolerance");
    assert_eq!(admission.reserved_cost, amount("3.999999999999"));
    let outside = vec![entity(
        USER,
        "10",
        "6.000000000002",
        CounterEntityType::User,
    )];
    let fake = seeded(&[(USER, "6.000000000002")], &[]);
    let strict_request = AdmissionRequest {
        fail_closed_budget_enforcement: true,
        ..request(&outside, "4")
    };
    let error = admit(&fake, &strict_request, &mut Vec::new())
        .await
        .expect_err("the strict rejection");
    let AdmissionError::BudgetExceeded { details } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(details.current_cost, amount("6.000000000002"));
}

// the resize lands on the headroom the counter had before the reservation, not a rounded figure
async fn a_resize_reaches_exactly_the_remaining_headroom() {
    let counters = vec![entity(USER, "10", "6.5", CounterEntityType::User)];
    let fake = seeded(&[(USER, "6.5")], &[]);
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "5"), &mut applied)
        .await
        .expect("the resize");
    assert_eq!(admission.reserved_cost, amount("3.5"));
    assert_eq!(applied[0].reserved_cost, Some(amount("3.5")));
    assert_eq!(value_of(&fake, USER), Some(amount("10")));
    let calls = take_calls(&fake);
    assert_eq!(
        calls.settlements,
        vec![vec![(USER.to_owned(), amount("-1.5"))]]
    );
}

// a pipeline answer shorter than the group leaves the rest of it on the pre-read spend
async fn short_pipeline_answers_fall_back_to_the_pre_read() {
    let counters = vec![entity(KEY, "5", "2", CounterEntityType::Key)];
    let mut fake = seeded(&[(KEY, "2")], &[]);
    fake.dropped_results = 1;
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect("the resize");
    assert_eq!(admission.reserved_cost, amount("3"));
    assert_eq!(value_of(&fake, KEY), Some(amount("5")));
    let calls = take_calls(&fake);
    assert_eq!(
        calls.reservations,
        vec![vec![(KEY.to_owned(), amount("4"))]]
    );
    assert_eq!(
        calls.settlements,
        vec![vec![(KEY.to_owned(), amount("-1"))]]
    );
}

// every counter skipped at initialization leaves the estimate standing with nothing charged
async fn an_empty_reservable_list_returns_the_estimate() {
    let counters = vec![entity(USER, "5", "0", CounterEntityType::User)];
    let mut fake = seeded(&[], &[]);
    let _inserted = fake.failed_seeds.insert(USER.to_owned());
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect("the admission");
    assert_eq!(admission.reserved_cost, amount("4"));
    let calls = take_calls(&fake);
    assert!(calls.reservations.is_empty());
    assert!(applied.is_empty());
}

// a counter whose entry the group lost is passed over, and the counters after it still settle
async fn a_lost_entry_does_not_stop_later_counters() {
    let counters = vec![
        entity(USER, "5", "9", CounterEntityType::User),
        entity(KEY, "8", "10", CounterEntityType::Key),
    ];
    let references: Vec<&BudgetCounter> = counters.iter().collect();
    let mut current_spend = BTreeMap::new();
    let _previous = current_spend.insert(KEY.to_owned(), amount("10"));
    let fake = seeded(&[(KEY, "10")], &[]);
    let mut applied = vec![counter_to_reservation_entry(&counters[1], amount("4"))];
    let charge = GroupCharge {
        counters: &references,
        current_spend: &current_spend,
        cost: amount("4"),
        reserved_values: vec![Some(amount("9")), Some(amount("10"))],
    };
    let cost = settle_group(&fake, &request(&counters, "4"), &charge, &mut applied)
        .await
        .expect("the settle");
    assert_eq!(cost, amount("2"));
    let calls = take_calls(&fake);
    assert_eq!(
        calls.settlements,
        vec![vec![(KEY.to_owned(), amount("-2"))]]
    );
}

// a counter whose reported value landed without a pre-read is passed over the same way
async fn a_lost_pre_read_does_not_stop_later_counters() {
    let counters = vec![
        entity(USER, "5", "9", CounterEntityType::User),
        entity(KEY, "8", "10", CounterEntityType::Key),
    ];
    let references: Vec<&BudgetCounter> = counters.iter().collect();
    let mut current_spend = BTreeMap::new();
    let _previous = current_spend.insert(KEY.to_owned(), amount("10"));
    let fake = seeded(&[(USER, "5"), (KEY, "10")], &[]);
    let mut applied = vec![
        counter_to_reservation_entry(&counters[0], amount("4")),
        counter_to_reservation_entry(&counters[1], amount("4")),
    ];
    let charge = GroupCharge {
        counters: &references,
        current_spend: &current_spend,
        cost: amount("4"),
        reserved_values: vec![None, Some(amount("10"))],
    };
    let cost = settle_group(&fake, &request(&counters, "4"), &charge, &mut applied)
        .await
        .expect("the settle");
    assert_eq!(cost, amount("2"));
    let calls = take_calls(&fake);
    assert_eq!(
        calls.settlements,
        vec![vec![
            (USER.to_owned(), amount("-2")),
            (KEY.to_owned(), amount("-2")),
        ]]
    );
    assert_eq!(value_of(&fake, USER), Some(amount("3")));
}

#[tokio::test]
async fn reservation_admission_engine_boundaries() {
    a_cold_window_is_skipped_without_invalidating().await;
    a_failed_window_seed_invalidates_and_rejects().await;
    a_rejection_stops_before_later_groups().await;
    a_failed_pipeline_rolls_back_only_its_group().await;
    the_remaining_budget_tolerance_decides().await;
    the_strict_overshoot_tolerance_decides().await;
    a_resize_reaches_exactly_the_remaining_headroom().await;
    short_pipeline_answers_fall_back_to_the_pre_read().await;
    an_empty_reservable_list_returns_the_estimate().await;
    a_lost_entry_does_not_stop_later_counters().await;
    a_lost_pre_read_does_not_stop_later_counters().await;
}
