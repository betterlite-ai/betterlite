#![cfg(test)]

use std::collections::BTreeSet;

use crate::reconciliation_operations::tests::support::{
    ITPM_SCOPE, OTPM_SCOPE, counter_key, io_ops, io_ops_for, io_reconciliation, reservation_tuples,
    scopes, slot_from, usage,
};
use crate::request_stash::{RequestStashSlot, StashState};
use crate::usage_resolution::ResponseObject;

fn reserved_slot() -> RequestStashSlot {
    slot_from(StashState {
        itpm_reserved_tokens: 40,
        itpm_reserved_scopes: scopes(&[ITPM_SCOPE]),
        otpm_reserved_tokens: 20,
        otpm_reserved_scopes: scopes(&[OTPM_SCOPE]),
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: 0,
        reserved_model: None,
        reserved_scopes: BTreeSet::new(),
        itpm_reserved_windows: BTreeSet::new(),
        otpm_reserved_windows: BTreeSet::new(),
        batch_enqueued_reservation: None,
        batch_tpd_refund_ops: Vec::new(),
        reservation_released: false,
        tpm_limited_tags: BTreeSet::new(),
    })
}

#[test]
fn a_slot_without_a_stash_builds_no_operations() {
    let response = ResponseObject::Usage(usage(100, 50));
    let reconciliation = io_reconciliation(&response, None);
    assert!(io_ops(&RequestStashSlot::default(), &reconciliation).is_empty());
}

#[test]
fn a_callback_for_another_call_builds_no_operations() {
    let slot = reserved_slot();
    let stash = slot.claim_for_call_id(Some("call-1"));
    assert_eq!(
        stash
            .state
            .lock()
            .expect("the state lock is free")
            .owner_litellm_call_id
            .as_deref(),
        Some("call-1")
    );
    let response = ResponseObject::Usage(usage(100, 50));
    let reconciliation = io_reconciliation(&response, None);
    assert!(io_ops_for(&slot, Some("call-2"), &reconciliation).is_empty());
}

#[test]
fn a_callback_for_the_owning_call_reconciles() {
    let slot = reserved_slot();
    let stash = slot.claim_for_call_id(Some("call-1"));
    assert_eq!(
        stash
            .state
            .lock()
            .expect("the state lock is free")
            .owner_litellm_call_id
            .as_deref(),
        Some("call-1")
    );
    let response = ResponseObject::Usage(usage(100, 50));
    let reconciliation = io_reconciliation(&response, None);
    assert_eq!(
        reservation_tuples(&io_ops_for(&slot, Some("call-1"), &reconciliation)),
        vec![
            (
                counter_key(ITPM_SCOPE),
                60.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                30.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}
