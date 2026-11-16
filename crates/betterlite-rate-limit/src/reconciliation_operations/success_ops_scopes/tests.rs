#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;

use crate::reconciliation_operations::scope_targets::ScopeMetadata;
use crate::reconciliation_operations::tests::support::{
    SuccessParts, SuccessResponseFixture, logging_metadata, scopes, slot_from, success_increments,
    tags, usage,
};
use crate::request_stash::StashState;
use crate::usage_resolution::ResponseObject;

fn parts_with(state: StashState) -> SuccessParts {
    SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        logging: logging_metadata(),
        slot: slot_from(state),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    }
}

#[test]
fn reserved_agent_scopes_are_appended_to_the_logging_targets() {
    let state = StashState {
        reserved_scopes: scopes(&[
            ("agent", "agent-1"),
            ("agent_session", "agent-1:sess-1"),
            ("team", "team-9"),
        ]),
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: 0,
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
    assert_eq!(
        success_increments(&parts_with(state)),
        vec![
            ("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{agent:agent-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            (
                "{agent_session:agent-1:sess-1}:tokens".to_owned(),
                15.0_f64,
                Some(60)
            ),
        ]
    );
}

#[test]
fn a_reserved_agent_scope_already_in_the_targets_is_not_duplicated() {
    let state = StashState {
        reserved_scopes: scopes(&[("agent", "agent-1"), ("agent_session", "agent-1:sess-1")]),
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: 0,
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
    let parts = SuccessParts {
        logging: ScopeMetadata {
            agent_id: Some("agent-1".to_owned()),
            session_id: Some("sess-1".to_owned()),
            ..logging_metadata()
        },
        ..parts_with(state)
    };
    assert_eq!(
        success_increments(&parts),
        vec![
            ("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{agent:agent-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            (
                "{agent_session:agent-1:sess-1}:tokens".to_owned(),
                15.0_f64,
                Some(60)
            ),
        ]
    );
}

#[test]
fn the_request_user_names_the_end_user_scope() {
    let parts = SuccessParts {
        request_user: Some("caller".to_owned()),
        ..parts_with(StashState::default())
    };
    assert_eq!(
        success_increments(&parts),
        vec![
            ("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{end_user:caller}:tokens".to_owned(), 15.0_f64, Some(60)),
        ]
    );
}

#[test]
fn stash_tags_are_charged_in_sorted_order() {
    let state = StashState {
        tpm_limited_tags: tags(&["beta", "alpha"]),
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: 0,
        reserved_model: None,
        reserved_scopes: BTreeSet::new(),
        itpm_reserved_tokens: 0,
        itpm_reserved_scopes: BTreeSet::new(),
        itpm_reserved_windows: BTreeSet::new(),
        otpm_reserved_tokens: 0,
        otpm_reserved_scopes: BTreeSet::new(),
        otpm_reserved_windows: BTreeSet::new(),
        batch_enqueued_reservation: None,
        batch_tpd_refund_ops: Vec::new(),
        reservation_released: false,
    };
    assert_eq!(
        success_increments(&parts_with(state)),
        vec![
            ("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{tag:alpha}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{tag:beta}:tokens".to_owned(), 15.0_f64, Some(60)),
        ]
    );
}
