#![cfg(test)]

use std::collections::BTreeSet;

use crate::reconciliation_operations::tests::support::{
    ITPM_SCOPE, OTPM_SCOPE, counter_key, io_ops, io_reconciliation, io_rerank, reservation_tuples,
    scopes, slot_from, total_only, usage,
};
use crate::request_stash::{RequestStashSlot, StashState};
use crate::usage_resolution::{ResponseAPIUsage, ResponseObject};

fn reserved_slot() -> RequestStashSlot {
    state_slot(false)
}

fn released_slot() -> RequestStashSlot {
    state_slot(true)
}

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
fn response_usage_is_reconciled_before_combined_usage() {
    let slot = reserved_slot();
    let response = ResponseObject::Usage(usage(100, 50));
    let combined = usage(900, 900);
    assert_eq!(
        reservation_tuples(&io_ops(
            &slot,
            &io_reconciliation(&response, Some(&combined))
        )),
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
fn combined_usage_is_reconciled_when_the_response_has_none() {
    let slot = reserved_slot();
    let response = ResponseObject::WithUsage(None);
    let combined = usage(100, 50);
    assert_eq!(
        reservation_tuples(&io_ops(
            &slot,
            &io_reconciliation(&response, Some(&combined))
        )),
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
fn a_response_without_usage_and_without_combined_usage_builds_nothing() {
    let slot = reserved_slot();
    let response = ResponseObject::WithUsage(None);
    assert!(io_ops(&slot, &io_reconciliation(&response, None)).is_empty());
}

#[test]
fn aggregate_only_total_charges_both_sides() {
    let slot = reserved_slot();
    let response = ResponseObject::Usage(total_only(90));
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
        vec![
            (
                counter_key(ITPM_SCOPE),
                50.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                70.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}

#[test]
fn a_response_aggregate_wins_over_a_combined_aggregate() {
    let slot = reserved_slot();
    let response = ResponseObject::Usage(total_only(90));
    let combined = total_only(120);
    assert_eq!(
        reservation_tuples(&io_ops(
            &slot,
            &io_reconciliation(&response, Some(&combined))
        )),
        vec![
            (
                counter_key(ITPM_SCOPE),
                50.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                70.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}

#[test]
fn a_combined_aggregate_is_used_when_the_response_has_none() {
    let slot = reserved_slot();
    let response = ResponseObject::WithUsage(None);
    let combined = total_only(120);
    assert_eq!(
        reservation_tuples(&io_ops(
            &slot,
            &io_reconciliation(&response, Some(&combined))
        )),
        vec![
            (
                counter_key(ITPM_SCOPE),
                80.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                100.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}

#[test]
fn missing_usage_on_a_released_stash_falls_back_to_the_reserved_counts() {
    let slot = released_slot();
    let response = ResponseObject::WithUsage(None);
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
fn rerank_tokens_are_reconciled() {
    let slot = reserved_slot();
    let reconciliation = io_rerank(Some((30, 0)), None);
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &reconciliation)),
        vec![
            (
                counter_key(ITPM_SCOPE),
                -10.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                -20.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}

#[test]
fn rerank_billed_units_total_is_used_when_tokens_are_missing() {
    let slot = reserved_slot();
    let reconciliation = io_rerank(None, Some(25));
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &reconciliation)),
        vec![
            (
                counter_key(ITPM_SCOPE),
                -15.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                -20.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}

#[test]
fn rerank_usage_without_counts_builds_nothing() {
    let slot = reserved_slot();
    let reconciliation = io_rerank(Some((0, 0)), Some(0));
    assert!(io_ops(&slot, &reconciliation).is_empty());
}

#[test]
fn response_api_usage_is_reconciled() {
    let slot = reserved_slot();
    let response = ResponseObject::ResponseApi(ResponseAPIUsage {
        input_tokens: 70,
        output_tokens: 30,
        total_tokens: 0,
        input_tokens_details: None,
    });
    assert_eq!(
        reservation_tuples(&io_ops(&slot, &io_reconciliation(&response, None))),
        vec![
            (
                counter_key(ITPM_SCOPE),
                30.0_f64,
                Some(60),
                None,
                None,
                None
            ),
            (
                counter_key(OTPM_SCOPE),
                10.0_f64,
                Some(60),
                None,
                None,
                None
            ),
        ]
    );
}
