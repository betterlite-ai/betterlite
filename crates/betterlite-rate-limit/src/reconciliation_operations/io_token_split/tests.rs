#![cfg(test)]

use std::collections::BTreeSet;

use crate::reconciliation_operations::tests::support::{
    ITPM_SCOPE, OTPM_SCOPE, counter_key, io_ops, io_reconciliation, reservation_tuples, scopes,
    slot_from, usage,
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
fn each_side_settles_its_own_token_count() {
    let slot = reserved_slot();
    let response = ResponseObject::Usage(usage(100, 50));
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
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

#[test]
fn every_input_scope_gets_the_same_input_delta() {
    let slot = slot_from(StashState {
        itpm_reserved_tokens: 40,
        itpm_reserved_scopes: scopes(&[ITPM_SCOPE, ("model_per_key", "key-1:gpt-4")]),
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
    });
    let response = ResponseObject::Usage(usage(100, 50));
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
        vec![
            (
                counter_key(("model_per_key", "key-1:gpt-4")),
                60.0_f64,
                Some(60),
                None,
                None,
                None
            ),
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
