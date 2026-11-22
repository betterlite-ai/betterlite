use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;
use crate::reservation_admission::engine::{AdmissionRequest, admit};
use crate::reservation_admission::error::AdmissionError;
use crate::reservation_settlement::entry::ReservationEntry;

use super::support::{
    KEY, USER, WINDOW, amount, entity, request, seeded, take_calls, token, value_of, window,
};

fn entry_keys(applied: &[ReservationEntry]) -> Vec<String> {
    applied
        .iter()
        .filter_map(|entry| entry.counter_key.clone())
        .collect()
}

// the counters whose spend loaded are charged together when every one of them has room for the estimate
async fn initialized_counters_charge_as_one_group() {
    let counters = vec![
        entity(KEY, "10", "0", CounterEntityType::Key),
        window(WINDOW, "5"),
        entity(USER, "20", "0", CounterEntityType::User),
    ];
    let fake = seeded(&[(WINDOW, "0.5")], &[(KEY, "1.5")]);
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect("the admission");
    assert_eq!(admission.reserved_cost, amount("4"));
    let calls = take_calls(&fake);
    assert_eq!(
        calls.seeds,
        vec![
            (KEY.to_owned(), KEY.to_owned()),
            (USER.to_owned(), USER.to_owned()),
        ]
    );
    assert_eq!(calls.windows, vec![WINDOW.to_owned()]);
    assert_eq!(
        calls.reservations,
        vec![vec![
            (KEY.to_owned(), amount("4")),
            (WINDOW.to_owned(), amount("4")),
            (USER.to_owned(), amount("4")),
        ]]
    );
    assert!(calls.settlements.is_empty());
    assert!(calls.invalidations.is_empty());
    assert_eq!(value_of(&fake, KEY), Some(amount("5.5")));
    assert_eq!(value_of(&fake, WINDOW), Some(amount("4.5")));
    assert_eq!(value_of(&fake, USER), Some(amount("4")));
    assert_eq!(entry_keys(&applied), vec![KEY, WINDOW, USER]);
}

// a counter whose spend cannot be loaded is dropped, and strict enforcement rejects the request instead
async fn unavailable_counters_skip_or_reject() {
    let counters = vec![
        entity(KEY, "10", "1", CounterEntityType::Key),
        entity(USER, "5", "0", CounterEntityType::User),
    ];
    let mut fake = seeded(&[(KEY, "1")], &[]);
    let _inserted = fake.failed_seeds.insert(USER.to_owned());
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect("the admission");
    assert_eq!(admission.reserved_cost, amount("4"));
    assert_eq!(entry_keys(&applied), vec![KEY]);
    assert_eq!(value_of(&fake, KEY), Some(amount("5")));
    let calls = take_calls(&fake);
    assert_eq!(calls.invalidations, vec![USER.to_owned()]);
    assert_eq!(
        calls.reservations,
        vec![vec![(KEY.to_owned(), amount("4"))]]
    );
    let mut strict = seeded(&[(KEY, "1")], &[]);
    let _inserted = strict.failed_seeds.insert(USER.to_owned());
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
    assert_eq!(value_of(&strict, KEY), Some(amount("1")));
    assert!(rejected.is_empty());
}

// an over-budget key that opted into throttling releases its own reservation and keeps the cost
async fn throttle_release_frees_only_the_key_counter() {
    let counters = vec![
        entity(KEY, "10", "9", CounterEntityType::Key),
        entity(USER, "20", "0", CounterEntityType::User),
    ];
    let fake = seeded(&[(KEY, "9")], &[]);
    let auth = token(true);
    let mut request_value = request(&counters, "4");
    request_value.token = Some(&auth);
    request_value.throttle_percentage = Some(0.5);
    let mut applied = Vec::new();
    let admission = admit(&fake, &request_value, &mut applied)
        .await
        .expect("the admission");
    assert_eq!(admission.reserved_cost, amount("4"));
    assert_eq!(entry_keys(&applied), vec![USER]);
    assert_eq!(value_of(&fake, KEY), Some(amount("9")));
    assert_eq!(value_of(&fake, USER), Some(amount("4")));
    let calls = take_calls(&fake);
    assert_eq!(
        calls.reservations,
        vec![
            vec![(KEY.to_owned(), amount("4"))],
            vec![(USER.to_owned(), amount("4"))],
        ]
    );
    assert_eq!(
        calls.settlements,
        vec![vec![(KEY.to_owned(), amount("-4"))]]
    );
}

// a throttle-opted token does not exempt any counter but the key's own max_budget one
async fn a_throttled_token_still_blocks_a_user_cap() {
    let counters = vec![entity(USER, "5", "5", CounterEntityType::User)];
    let fake = seeded(&[(USER, "5")], &[]);
    let auth = token(true);
    let mut request_value = request(&counters, "4");
    request_value.token = Some(&auth);
    request_value.throttle_percentage = Some(0.5);
    let mut applied = Vec::new();
    let error = admit(&fake, &request_value, &mut applied)
        .await
        .expect_err("the block");
    let AdmissionError::BudgetExceeded { details } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(details.current_cost, amount("9"));
    assert_eq!(details.max_budget, amount("5"));
    assert_eq!(details.entity_type.as_deref(), Some("user"));
    assert_eq!(details.entity_id.as_deref(), Some(USER));
    assert_eq!(error.status_code(), Some(422));
    assert_eq!(value_of(&fake, USER), Some(amount("9")));
}

// positive headroom that cannot hold the estimate resizes every applied entry down to it
async fn a_partial_headroom_resizes_the_reservation() {
    let counters = vec![entity(USER, "10", "7", CounterEntityType::User)];
    let fake = seeded(&[(USER, "7")], &[]);
    let mut applied = Vec::new();
    let admission = admit(&fake, &request(&counters, "5"), &mut applied)
        .await
        .expect("the resize");
    assert_eq!(admission.reserved_cost, amount("3"));
    assert_eq!(applied[0].reserved_cost, Some(amount("3")));
    assert_eq!(applied[0].applied_adjustment, Decimal::ZERO);
    assert_eq!(value_of(&fake, USER), Some(amount("10")));
    let calls = take_calls(&fake);
    assert_eq!(
        calls.settlements,
        vec![vec![(USER.to_owned(), amount("-2"))]]
    );
}

// no headroom at all blocks, since there is nothing left to resize to
async fn a_spent_budget_blocks_the_reservation() {
    let counters = vec![entity(USER, "5", "5", CounterEntityType::User)];
    let fake = seeded(&[(USER, "5")], &[]);
    let mut applied = Vec::new();
    let error = admit(&fake, &request(&counters, "4"), &mut applied)
        .await
        .expect_err("the block");
    let AdmissionError::BudgetExceeded { details } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(details.current_cost, amount("9"));
    assert_eq!(details.max_budget, amount("5"));
    assert_eq!(entry_keys(&applied), vec![USER]);
}

// strict enforcement refuses an estimate the counter already cannot fit instead of resizing it
async fn strict_enforcement_rejects_an_estimate_that_cannot_fit() {
    let counters = vec![entity(USER, "10", "7", CounterEntityType::User)];
    let fake = seeded(&[(USER, "7")], &[]);
    let mut applied = Vec::new();
    let strict_request = AdmissionRequest {
        fail_closed_budget_enforcement: true,
        ..request(&counters, "5")
    };
    let error = admit(&fake, &strict_request, &mut applied)
        .await
        .expect_err("the rejection");
    let AdmissionError::BudgetExceeded { details } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(details.current_cost, amount("7"));
    assert_eq!(
        details.message.as_deref(),
        Some(
            "Budget has been exceeded! User=spend:user:u1 Current cost: 7, Estimated request cost: 5, Max budget: 10"
        )
    );
    assert_eq!(value_of(&fake, USER), Some(amount("12")));
}

// a reservation that lands exactly on the budget never reaches the over-budget policy, not even a throttled key's
async fn a_reservation_inside_the_budget_charges_without_policy() {
    let counters = vec![
        entity(KEY, "10", "6", CounterEntityType::Key),
        entity(USER, "10", "6", CounterEntityType::User),
    ];
    let fake = seeded(&[(KEY, "6"), (USER, "6")], &[]);
    let auth = token(true);
    let mut request_value = request(&counters, "4");
    request_value.token = Some(&auth);
    request_value.throttle_percentage = Some(0.5);
    let mut applied = Vec::new();
    let admission = admit(&fake, &request_value, &mut applied)
        .await
        .expect("the admission");
    assert_eq!(admission.reserved_cost, amount("4"));
    assert_eq!(entry_keys(&applied), vec![KEY, USER]);
    assert_eq!(value_of(&fake, KEY), Some(amount("10")));
    assert_eq!(applied[0].reserved_cost, Some(amount("4")));
    let calls = take_calls(&fake);
    assert!(calls.settlements.is_empty());
    assert!(calls.invalidations.is_empty());
    assert!(calls.cache_reads.is_empty());
}

#[tokio::test]
async fn reservation_admission_engine_behaviors() {
    initialized_counters_charge_as_one_group().await;
    unavailable_counters_skip_or_reject().await;
    throttle_release_frees_only_the_key_counter().await;
    a_throttled_token_still_blocks_a_user_cap().await;
    a_partial_headroom_resizes_the_reservation().await;
    a_spent_budget_blocks_the_reservation().await;
    strict_enforcement_rejects_an_estimate_that_cannot_fit().await;
    a_reservation_inside_the_budget_charges_without_policy().await;
}
