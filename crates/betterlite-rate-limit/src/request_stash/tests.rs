#![cfg(test)]

use std::sync::{Arc, MutexGuard};

use rstest::rstest;
use tokio::sync::Barrier;
use tokio::task::JoinSet;

use super::{RequestStash, RequestStashSlot, StashState};

fn state_lock(stash: &RequestStash) -> MutexGuard<'_, StashState> {
    stash.state.lock().expect("the state lock is not poisoned")
}

fn owned(call_id: &str) -> RequestStashSlot {
    let slot = RequestStashSlot::default();
    let claimed = slot.claim_for_call_id(Some(call_id));
    assert_eq!(
        state_lock(&claimed).owner_litellm_call_id.as_deref(),
        Some(call_id)
    );
    slot
}

fn mark_released(stash: &RequestStash) {
    let mut state = state_lock(stash);
    state.reservation_released = true;
}

fn set_owner(stash: &RequestStash, owner: Option<&str>) {
    let mut state = state_lock(stash);
    state.owner_litellm_call_id = owner.map(str::to_owned);
}

#[test]
fn stash_starts_with_the_token_and_model_defaults() {
    let state = StashState::default();
    assert_eq!(state.owner_litellm_call_id, None);
    assert_eq!(state.rate_limit_response, None);
    assert_eq!(state.parallel_slot, None);
    assert_eq!(state.reserved_tokens, 0);
    assert_eq!(state.reserved_model, None);
    assert!(state.reserved_scopes.is_empty());
    assert_eq!(state.itpm_reserved_tokens, 0);
    assert!(state.itpm_reserved_scopes.is_empty());
    assert!(state.itpm_reserved_windows.is_empty());
}

#[test]
fn stash_starts_with_the_otpm_and_callback_defaults() {
    let state = StashState::default();
    assert_eq!(state.otpm_reserved_tokens, 0);
    assert!(state.otpm_reserved_scopes.is_empty());
    assert!(state.otpm_reserved_windows.is_empty());
    assert_eq!(state.batch_enqueued_reservation, None);
    assert!(state.batch_tpd_refund_ops.is_empty());
    assert!(!state.reservation_released);
    assert!(state.tpm_limited_tags.is_empty());
}

#[test]
fn a_new_slot_holds_no_stash() {
    let slot = RequestStashSlot::default();
    assert!(slot.get().is_none());
    assert!(slot.for_call(None).is_none());
    assert!(slot.for_call(Some("call-1")).is_none());
}

#[test]
fn get_or_create_returns_the_same_stash_on_every_call() {
    let slot = RequestStashSlot::default();
    let first = slot.get_or_create();
    let second = slot.get_or_create();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first, &slot.get().expect("the stash is set")));
}

#[test]
fn claim_for_call_id_sets_the_owner_only_for_a_string() {
    let slot = RequestStashSlot::default();
    let stash = slot.claim_for_call_id(Some("call-1"));
    assert_eq!(
        state_lock(&stash).owner_litellm_call_id.as_deref(),
        Some("call-1")
    );
    let kept = slot.claim_for_call_id(None);
    assert_eq!(
        state_lock(&kept).owner_litellm_call_id.as_deref(),
        Some("call-1")
    );
    let replaced = slot.claim_for_call_id(Some("call-2"));
    assert_eq!(
        state_lock(&replaced).owner_litellm_call_id.as_deref(),
        Some("call-2")
    );
}

#[rstest]
#[case::owner_unset_and_call_id_given(None, Some("call-1"), true)]
#[case::owner_unset_and_call_id_missing(None, None, true)]
#[case::owner_and_call_id_match(Some("call-1"), Some("call-1"), true)]
#[case::owner_and_call_id_differ(Some("call-1"), Some("call-2"), false)]
#[case::owner_set_and_call_id_missing(Some("call-1"), None, true)]
fn for_call_reads_a_stash_by_its_owner(
    #[case] owner: Option<&str>,
    #[case] call_id: Option<&str>,
    #[case] answered: bool,
) {
    let slot = RequestStashSlot::default();
    let stash = slot.get_or_create();
    set_owner(&stash, owner);
    assert_eq!(slot.for_call(call_id).is_some(), answered);
}

#[test]
fn a_nested_call_without_a_slot_sees_no_stash() {
    let owner_slot = owned("owner-call");
    let nested_slot = RequestStashSlot::default();
    assert!(nested_slot.get().is_none());
    assert!(nested_slot.for_call(Some("owner-call")).is_none());
    assert!(owner_slot.for_call(Some("nested-call")).is_none());
}

#[tokio::test]
async fn the_state_lock_and_the_slot_release_lock_are_separate() {
    let stash = RequestStashSlot::default().get_or_create();
    let release = stash.parallel_slot_release_lock.lock().await;
    mark_released(&stash);
    assert!(state_lock(&stash).reservation_released);
    drop(release);
    let free = stash
        .parallel_slot_release_lock
        .try_lock()
        .expect("the release lock is free again");
    drop(free);
}

#[tokio::test]
async fn clones_of_the_slot_share_one_stash() {
    let slot = RequestStashSlot::default();
    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = JoinSet::new();
    let first = slot.clone();
    let first_barrier = Arc::clone(&barrier);
    tasks.spawn(async move {
        let stash = first.get_or_create();
        mark_released(&stash);
        first_barrier.wait().await;
        assert!(state_lock(&stash).reservation_released);
    });
    let second = slot.clone();
    let second_barrier = Arc::clone(&barrier);
    tasks.spawn(async move {
        let stash = second.get_or_create();
        second_barrier.wait().await;
        assert!(state_lock(&stash).reservation_released);
    });
    while let Some(result) = tasks.join_next().await {
        result.expect("the task ran to completion");
    }
}
