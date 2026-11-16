#![cfg(test)]

use std::collections::BTreeSet;

use crate::reconciliation_operations::tests::support::{
    ITPM_SCOPE, OTPM_SCOPE, counter_key, io_ops, io_reconciliation, reservation_tuples, scopes,
    slot_from, usage,
};
use crate::request_stash::{RequestStashSlot, StashState};
use crate::usage_resolution::{ResponseObject, Usage};

fn state_slot(released: bool) -> RequestStashSlot {
    slot_from(StashState {
        itpm_reserved_tokens: 40,
        itpm_reserved_scopes: scopes(&[ITPM_SCOPE]),
        otpm_reserved_tokens: 20,
        otpm_reserved_scopes: scopes(&[OTPM_SCOPE]),
        reservation_released: released,
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
        tpm_limited_tags: BTreeSet::new(),
    })
}

#[test]
fn zero_response_usage_builds_nothing() {
    let slot = state_slot(false);
    let response = ResponseObject::Usage(Usage::default());
    assert!(io_ops(&slot, &io_reconciliation(&response, None)).is_empty());
}

#[test]
fn zero_combined_usage_builds_nothing() {
    let slot = state_slot(false);
    let response = ResponseObject::WithUsage(None);
    let combined = Usage::default();
    assert!(io_ops(&slot, &io_reconciliation(&response, Some(&combined))).is_empty());
}

#[test]
fn a_released_stash_charges_the_reserved_counts_on_zero_usage() {
    let slot = state_slot(true);
    let response = ResponseObject::Usage(Usage::default());
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
        vec![
            (
                counter_key(ITPM_SCOPE),
                40.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                20.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}

#[test]
fn zero_reserved_counts_with_recorded_scopes_build_nothing() {
    let slot = slot_from(StashState {
        itpm_reserved_tokens: 0,
        itpm_reserved_scopes: scopes(&[ITPM_SCOPE]),
        otpm_reserved_tokens: 0,
        otpm_reserved_scopes: scopes(&[OTPM_SCOPE]),
        reservation_released: false,
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
        tpm_limited_tags: BTreeSet::new(),
    });
    let response = ResponseObject::Usage(usage(100, 50));
    assert!(io_ops(&slot, &io_reconciliation(&response, None)).is_empty());
}
