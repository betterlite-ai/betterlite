#![cfg(test)]

use std::collections::BTreeSet;

use crate::limiter_records::ReservationBackend;
use crate::reconciliation_operations::tests::support::{
    ITPM_SCOPE, OTPM_SCOPE, counter_key, io_ops, io_reconciliation, reservation_tuples, scopes,
    slot_from, usage, window,
};
use crate::request_stash::StashState;
use crate::usage_resolution::ResponseObject;

fn windowed_state(released: bool) -> StashState {
    StashState {
        itpm_reserved_tokens: 40,
        itpm_reserved_scopes: scopes(&[ITPM_SCOPE]),
        itpm_reserved_windows: BTreeSet::from([window(ITPM_SCOPE, "1000")]),
        otpm_reserved_tokens: 20,
        otpm_reserved_scopes: scopes(&[OTPM_SCOPE]),
        otpm_reserved_windows: BTreeSet::from([window(OTPM_SCOPE, "1000")]),
        reservation_released: released,
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: 0,
        reserved_model: None,
        reserved_scopes: BTreeSet::new(),
        batch_enqueued_reservation: None,
        batch_tpd_refund_ops: Vec::new(),
        tpm_limited_tags: BTreeSet::new(),
    }
}

#[test]
fn a_released_stash_charges_the_full_actual_usage() {
    let slot = slot_from(windowed_state(true));
    let response = ResponseObject::Usage(usage(100, 50));
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
        vec![
            (
                counter_key(ITPM_SCOPE),
                100.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                50.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}

#[test]
fn an_unreleased_stash_with_windows_guards_every_scope() {
    let slot = slot_from(windowed_state(false));
    let response = ResponseObject::Usage(usage(100, 50));
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
        vec![
            (
                counter_key(ITPM_SCOPE),
                60.0_f64,
                Some(60),
                Some("{model_per_project_itpm:proj-1:gpt-4}:window".to_owned()),
                Some("1000".to_owned()),
                Some(ReservationBackend::Redis)
            ),
            (
                counter_key(OTPM_SCOPE),
                30.0_f64,
                Some(60),
                Some("{model_per_project_otpm:proj-1:gpt-4}:window".to_owned()),
                Some("1000".to_owned()),
                Some(ReservationBackend::Redis)
            ),
        ]
    );
}

#[test]
fn one_windowed_side_still_guards_every_scope() {
    let slot = slot_from(StashState {
        itpm_reserved_tokens: 40,
        itpm_reserved_scopes: scopes(&[ITPM_SCOPE]),
        itpm_reserved_windows: BTreeSet::new(),
        otpm_reserved_tokens: 20,
        otpm_reserved_scopes: scopes(&[OTPM_SCOPE]),
        otpm_reserved_windows: BTreeSet::from([window(OTPM_SCOPE, "1000")]),
        reservation_released: false,
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: 0,
        reserved_model: None,
        reserved_scopes: BTreeSet::new(),
        batch_enqueued_reservation: None,
        batch_tpd_refund_ops: Vec::new(),
        tpm_limited_tags: BTreeSet::new(),
    });
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
                Some("{model_per_project_otpm:proj-1:gpt-4}:window".to_owned()),
                Some("1000".to_owned()),
                Some(ReservationBackend::Redis)
            ),
        ]
    );
}

#[test]
fn a_zero_reserved_side_emits_no_operations() {
    let response = ResponseObject::Usage(usage(100, 50));
    let input_zero = slot_from(StashState {
        itpm_reserved_tokens: 0,
        ..windowed_state(false)
    });
    let output_zero = slot_from(StashState {
        otpm_reserved_tokens: 0,
        ..windowed_state(false)
    });
    assert_eq!(
        reservation_tuples(&io_ops(&input_zero, &io_reconciliation(&response, None))),
        vec![(
            counter_key(OTPM_SCOPE),
            30.0_f64,
            Some(60),
            Some("{model_per_project_otpm:proj-1:gpt-4}:window".to_owned()),
            Some("1000".to_owned()),
            Some(ReservationBackend::Redis)
        )]
    );
    assert_eq!(
        reservation_tuples(&io_ops(&output_zero, &io_reconciliation(&response, None))),
        vec![(
            counter_key(ITPM_SCOPE),
            60.0_f64,
            Some(60),
            Some("{model_per_project_itpm:proj-1:gpt-4}:window".to_owned()),
            Some("1000".to_owned()),
            Some(ReservationBackend::Redis)
        )]
    );
}

#[test]
fn zero_reserved_sides_with_windows_build_no_operations() {
    let slot = slot_from(StashState {
        itpm_reserved_tokens: 0,
        itpm_reserved_scopes: scopes(&[ITPM_SCOPE]),
        itpm_reserved_windows: BTreeSet::from([window(ITPM_SCOPE, "1000")]),
        otpm_reserved_tokens: 0,
        otpm_reserved_scopes: scopes(&[OTPM_SCOPE]),
        otpm_reserved_windows: BTreeSet::from([window(OTPM_SCOPE, "1000")]),
        reservation_released: false,
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: 0,
        reserved_model: None,
        reserved_scopes: BTreeSet::new(),
        batch_enqueued_reservation: None,
        batch_tpd_refund_ops: Vec::new(),
        tpm_limited_tags: BTreeSet::new(),
    });
    let response = ResponseObject::Usage(usage(100, 50));
    assert!(io_ops(&slot, &io_reconciliation(&response, None)).is_empty());
}
