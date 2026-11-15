#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;

use crate::reconciliation_operations::scope_targets::ScopeMetadata;
use crate::reconciliation_operations::tests::support::{
    SuccessParts, SuccessResponseFixture, logging_metadata, scopes, slot_from, success_increments,
    success_ops, usage,
};
use crate::request_stash::{RequestStashSlot, StashState};
use crate::usage_resolution::ResponseObject;

fn parts_with(state: StashState, call_id: Option<&str>) -> SuccessParts {
    SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        logging: logging_metadata(),
        slot: slot_from(state),
        call_id: call_id.map(str::to_owned),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        groups: BTreeMap::new(),
        window_size: 60,
    }
}

fn reserved_state() -> StashState {
    StashState {
        reserved_tokens: 100,
        reserved_scopes: scopes(&[("api_key", "key-1")]),
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_model: None,
        itpm_reserved_tokens: 0,
        itpm_reserved_scopes: BTreeSet::new(),
        itpm_reserved_windows: BTreeSet::new(),
        otpm_reserved_tokens: 0,
        otpm_reserved_scopes: BTreeSet::new(),
        otpm_reserved_windows: BTreeSet::new(),
        batch_enqueued_reservation: None,
        batch_tpd_refund_ops: Vec::new(),
        reservation_released: false,
        tpm_limited_tags: BTreeSet::new(),
    }
}

#[test]
fn a_reserved_scope_settles_to_actual_minus_reserved() {
    assert_eq!(
        success_increments(&parts_with(reserved_state(), None)),
        vec![("{api_key:key-1}:tokens".to_owned(), -85.0_f64, Some(60))]
    );
}

#[test]
fn without_a_reservation_every_scope_is_charged() {
    assert_eq!(
        success_increments(&parts_with(StashState::default(), None)),
        vec![("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60))]
    );
}

#[test]
fn a_zero_delta_scope_emits_no_operation() {
    let state = StashState {
        reserved_tokens: 15,
        reserved_scopes: scopes(&[("api_key", "key-1")]),
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_model: None,
        itpm_reserved_tokens: 0,
        itpm_reserved_scopes: BTreeSet::new(),
        itpm_reserved_windows: BTreeSet::new(),
        otpm_reserved_tokens: 0,
        otpm_reserved_scopes: BTreeSet::new(),
        otpm_reserved_windows: BTreeSet::new(),
        batch_enqueued_reservation: None,
        batch_tpd_refund_ops: Vec::new(),
        reservation_released: false,
        tpm_limited_tags: BTreeSet::new(),
    };
    assert!(success_increments(&parts_with(state, None)).is_empty());
}

#[test]
fn a_callback_for_another_call_sees_no_reservation() {
    let slot = slot_from(reserved_state());
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
    let base = parts_with(StashState::default(), None);
    let parts = SuccessParts {
        call_id: Some("call-2".to_owned()),
        slot,
        ..base
    };
    assert_eq!(
        success_increments(&parts),
        vec![("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60))]
    );
}

#[test]
fn a_request_without_targets_emits_nothing() {
    let parts = SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        logging: ScopeMetadata::default(),
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    };
    assert!(success_ops(&parts).expect("the ops build").is_empty());
}
