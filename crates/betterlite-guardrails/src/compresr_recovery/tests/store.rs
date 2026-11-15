use std::time::Duration;

use rstest::rstest;

use crate::compresr_recovery::store::{
    MAX_TOTAL_STORE_BYTES, MAX_TRACKED_CALLS, ORIGINALS_TTL, RecoveryStore, State, StoreKey,
};

use super::support::{CALL_ID, NO_CAP, NOW, SCOPE, key, originals, seeded};

fn after(count: u64) -> Duration {
    NOW.saturating_add(Duration::from_secs(count))
}

fn stored_hashes(store: &RecoveryStore, stored_key: &StoreKey) -> Vec<String> {
    store
        .existing_originals(Some(stored_key))
        .keys()
        .cloned()
        .collect()
}

#[test]
fn other_caller_cannot_retrieve_the_same_call_id_scoped() {
    let store = seeded(&[("h1", "original")]);
    let own = key(SCOPE, CALL_ID);
    let other = key("scope-b", CALL_ID);
    assert_eq!(
        store.retrieve(Some(&own), "h1", NOW),
        Some("original".to_owned())
    );
    assert_eq!(store.retrieve(Some(&other), "h1", NOW), None);
    assert!(store.existing_originals(Some(&other)).is_empty());
}

#[test]
fn other_call_id_of_the_same_caller_finds_nothing_scoped() {
    let store = seeded(&[("h1", "original")]);
    assert_eq!(store.retrieve(Some(&key(SCOPE, "call-2")), "h1", NOW), None);
}

#[test]
fn empty_scope_never_reads_scoped_entries_scoped() {
    let store = seeded(&[("h1", "original")]);
    assert_eq!(store.retrieve(Some(&key("", CALL_ID)), "h1", NOW), None);
}

#[test]
fn retrieve_without_a_key_finds_nothing_scoped() {
    let store = seeded(&[("h1", "original")]);
    assert_eq!(store.retrieve(None, "h1", NOW), None);
    assert!(store.existing_originals(None).is_empty());
}

#[rstest]
#[case::absent(None, false)]
#[case::empty(Some(""), false)]
#[case::present(Some("call-1"), true)]
fn call_id_decides_whether_recovery_has_a_key_scoped(
    #[case] call_id: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(StoreKey::resolve(call_id, SCOPE).is_some(), expected);
}

#[test]
fn later_turn_merges_into_the_same_key_scoped() {
    let store = seeded(&[("h1", "first"), ("h2", "second")]);
    let stored_key = key(SCOPE, CALL_ID);
    store.store(
        &stored_key,
        originals(&[("h3", "third"), ("h1", "replaced")]),
        NOW,
        NO_CAP,
    );
    assert_eq!(stored_hashes(&store, &stored_key), ["h1", "h2", "h3"]);
    assert_eq!(
        store.retrieve(Some(&stored_key), "h1", NOW),
        Some("replaced".to_owned())
    );
}

#[test]
fn entry_lives_until_exactly_fifteen_minutes_ttl() {
    let store = seeded(&[("h1", "original")]);
    let own = key(SCOPE, CALL_ID);
    let expiry = NOW.saturating_add(ORIGINALS_TTL);
    assert_eq!(ORIGINALS_TTL, Duration::from_mins(15));
    assert!(
        store
            .retrieve(
                Some(&own),
                "h1",
                expiry.saturating_sub(Duration::from_nanos(1))
            )
            .is_some()
    );
    assert_eq!(store.retrieve(Some(&own), "h1", expiry), None);
}

#[test]
fn storing_again_refreshes_the_expiry_ttl() {
    let store = seeded(&[("h1", "first")]);
    let own = key(SCOPE, CALL_ID);
    store.store(&own, originals(&[("h2", "second")]), after(600), NO_CAP);
    assert!(store.retrieve(Some(&own), "h1", after(1_499)).is_some());
    assert_eq!(store.retrieve(Some(&own), "h1", after(1_500)), None);
}

#[test]
fn pruning_drops_only_expired_front_entries_ttl() {
    let store = RecoveryStore::default();
    store.store(&key(SCOPE, "old"), originals(&[("h1", "aa")]), NOW, NO_CAP);
    store.store(
        &key(SCOPE, "mid"),
        originals(&[("h2", "bbb")]),
        after(300),
        NO_CAP,
    );
    assert_eq!(store.prune(after(899)), 0);
    assert_eq!(store.state.lock().unwrap().entries.len(), 2);
    assert_eq!(store.prune(after(900)), 1);
    let state = store.state.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.total_bytes, 3);
}

#[test]
fn storing_prunes_expired_entries_ttl() {
    let store = RecoveryStore::default();
    store.store(&key(SCOPE, "old"), originals(&[("h1", "aa")]), NOW, NO_CAP);
    store.store(
        &key(SCOPE, "new"),
        originals(&[("h2", "b")]),
        after(900),
        NO_CAP,
    );
    let state = store.state.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.total_bytes, 1);
}

#[rstest]
#[case::at_the_limit(256, 256)]
#[case::one_over(257, 256)]
fn oldest_call_leaves_past_the_call_limit_eviction(#[case] calls: usize, #[case] kept: usize) {
    assert_eq!(MAX_TRACKED_CALLS, 256);
    let store = RecoveryStore::default();
    for index in 0..calls {
        store.store(
            &key(SCOPE, &format!("call-{index}")),
            originals(&[("h", "x")]),
            NOW,
            NO_CAP,
        );
    }
    let (len, kept_first, kept_last) = {
        let state = store.state.lock().unwrap();
        (
            state.entries.len(),
            state.entries.contains_key(&key(SCOPE, "call-0")),
            state
                .entries
                .contains_key(&key(SCOPE, &format!("call-{}", calls.saturating_sub(1)))),
        )
    };
    assert_eq!(len, kept);
    assert_eq!(kept_first, calls == kept);
    assert!(kept_last);
}

#[rstest]
#[case::at_the_total(12, &["a", "b", "c"])]
#[case::one_drop_fits_exactly(8, &["b", "c"])]
#[case::one_byte_less_drops_two(7, &["c"])]
#[case::cap_off(0, &["a", "b", "c"])]
fn call_cap_drops_the_oldest_originals_eviction(#[case] cap: usize, #[case] kept: &[&str]) {
    let store = RecoveryStore::default();
    let own = key(SCOPE, CALL_ID);
    let batch = originals(&[("a", "1234"), ("b", "1234"), ("c", "1234")]);
    store.store(&own, batch, NOW, cap);
    assert_eq!(stored_hashes(&store, &own), kept);
}

#[test]
fn original_larger_than_the_call_cap_is_dropped_eviction() {
    let store = RecoveryStore::default();
    let own = key(SCOPE, CALL_ID);
    store.store(&own, originals(&[("a", "12345")]), NOW, 3);
    assert!(stored_hashes(&store, &own).is_empty());
    assert_eq!(store.state.lock().unwrap().total_bytes, 0);
}

#[test]
fn call_cap_counts_what_earlier_turns_stored_eviction() {
    let store = RecoveryStore::default();
    let own = key(SCOPE, CALL_ID);
    store.store(&own, originals(&[("a", "1234")]), NOW, 10);
    store.store(&own, originals(&[("b", "1234"), ("c", "1234")]), NOW, 10);
    assert_eq!(stored_hashes(&store, &own), ["b", "c"]);
}

#[test]
fn byte_total_follows_replacements_and_removals_eviction() {
    let store = RecoveryStore::default();
    let own = key(SCOPE, CALL_ID);
    store.store(&own, originals(&[("a", "1234"), ("b", "12")]), NOW, NO_CAP);
    assert_eq!(store.state.lock().unwrap().total_bytes, 6);
    store.store(&own, originals(&[("a", "1")]), NOW, NO_CAP);
    assert_eq!(store.state.lock().unwrap().total_bytes, 3);
    store.store(
        &key(SCOPE, "other"),
        originals(&[("c", "123")]),
        NOW,
        NO_CAP,
    );
    assert_eq!(store.state.lock().unwrap().total_bytes, 6);
    assert_eq!(store.prune(NOW.saturating_add(ORIGINALS_TTL)), 2);
    assert_eq!(store.state.lock().unwrap().total_bytes, 0);
}

#[rstest]
#[case::under_the_ceiling(&[6, 3], 10, 2)]
#[case::exactly_the_ceiling(&[6, 4], 10, 2)]
#[case::one_byte_over_drops_the_oldest(&[6, 5], 10, 1)]
#[case::the_newest_entry_is_kept_alone(&[20], 10, 1)]
#[case::oldest_go_first_until_it_fits(&[4, 4, 4], 9, 2)]
fn global_ceiling_evicts_oldest_but_keeps_the_newest_eviction(
    #[case] sizes: &[usize],
    #[case] ceiling: usize,
    #[case] kept: usize,
) {
    let mut state = State::default();
    for (index, size) in sizes.iter().enumerate() {
        let text = "x".repeat(*size);
        let entry = originals(&[("h", text.as_str())]);
        state.insert(&key(SCOPE, &format!("call-{index}")), entry, after(900));
    }
    state.prune(NOW, ceiling);
    assert_eq!(state.entries.len(), kept);
    assert!(state.entries.contains_key(&key(
        SCOPE,
        &format!("call-{}", sizes.len().saturating_sub(1))
    )));
    assert_eq!(MAX_TOTAL_STORE_BYTES, 256 * 1024 * 1024);
}
