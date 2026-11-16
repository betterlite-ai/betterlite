#![cfg(test)]

use std::collections::BTreeSet;

use crate::reconciliation_operations::tests::support::{
    ITPM_SCOPE, OTPM_SCOPE, counter_key, io_ops, io_reconciliation, reservation_tuples, scopes,
    slot_from, usage,
};
use crate::request_stash::StashState;
use crate::usage_resolution::ResponseObject;

fn reserved_state(input_tokens: i64, output_tokens: i64) -> StashState {
    StashState {
        itpm_reserved_tokens: input_tokens,
        itpm_reserved_scopes: if input_tokens > 0 {
            scopes(&[ITPM_SCOPE])
        } else {
            scopes(&[])
        },
        otpm_reserved_tokens: output_tokens,
        otpm_reserved_scopes: if output_tokens > 0 {
            scopes(&[OTPM_SCOPE])
        } else {
            scopes(&[])
        },
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
    }
}

#[test]
fn zero_reserved_counts_build_no_operations() {
    let slot = slot_from(reserved_state(0, 0));
    let response = ResponseObject::Usage(usage(100, 50));
    assert!(io_ops(&slot, &io_reconciliation(&response, None)).is_empty());
}

#[test]
fn negative_reserved_counts_build_no_operations() {
    let slot = slot_from(reserved_state(-5, -3));
    let response = ResponseObject::Usage(usage(100, 50));
    assert!(io_ops(&slot, &io_reconciliation(&response, None)).is_empty());
}

#[test]
fn an_input_reservation_alone_reconciles() {
    let slot = slot_from(reserved_state(40, 0));
    let response = ResponseObject::Usage(usage(100, 50));
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
        vec![(
            counter_key(ITPM_SCOPE),
            60.0_f64,
            Some(60),
            None,
            None,
            None
        )]
    );
}

#[test]
fn an_output_reservation_alone_reconciles() {
    let slot = slot_from(reserved_state(0, 20));
    let response = ResponseObject::Usage(usage(100, 50));
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
        vec![(
            counter_key(OTPM_SCOPE),
            30.0_f64,
            Some(60),
            None,
            None,
            None
        )]
    );
}
