#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;

use crate::limiter_records::RateLimitedModel;
use crate::reconciliation_operations::tests::support::{
    SuccessParts, SuccessResponseFixture, model_scope_metadata, slot_from, success_increments,
    usage,
};
use crate::request_stash::{RequestStashSlot, StashState};
use crate::usage_resolution::ResponseObject;

fn routed_parts() -> SuccessParts {
    SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        logging: model_scope_metadata(),
        model_group: Some("alias-name".to_owned()),
        groups: BTreeMap::from([("alias-name".to_owned(), "alias-group".to_owned())]),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        window_size: 60,
    }
}

fn reserved_parts(requested: &str, group: &str) -> StashState {
    StashState {
        reserved_model: Some(RateLimitedModel {
            requested: requested.to_owned(),
            group: group.to_owned(),
        }),
        owner_litellm_call_id: None,
        rate_limit_response: None,
        parallel_slot: None,
        reserved_tokens: 0,
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
        tpm_limited_tags: BTreeSet::new(),
    }
}

#[test]
fn without_a_reservation_the_router_group_names_the_model_scopes() {
    assert_eq!(
        success_increments(&routed_parts()),
        vec![
            ("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{team:team-1}:tokens".to_owned(), 15.0, Some(60)),
            ("{organization:org-1}:tokens".to_owned(), 15.0, Some(60)),
            (
                "{model_per_key:key-1:alias-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_team:team-1:alias-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_organization:org-1:alias-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_project:proj-1:alias-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
        ]
    );
}

#[test]
fn a_reserved_model_wins_over_the_router_group() {
    let parts = SuccessParts {
        slot: slot_from(reserved_parts("old-alias", "reserved-group")),
        ..routed_parts()
    };
    assert_eq!(
        success_increments(&parts),
        vec![
            ("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{team:team-1}:tokens".to_owned(), 15.0, Some(60)),
            ("{organization:org-1}:tokens".to_owned(), 15.0, Some(60)),
            (
                "{model_per_key:key-1:reserved-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_team:team-1:reserved-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_organization:org-1:reserved-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_project:proj-1:reserved-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
        ]
    );
}

#[test]
fn without_a_model_group_there_are_no_model_scopes() {
    let parts = SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        logging: model_scope_metadata(),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    };
    assert_eq!(
        success_increments(&parts),
        vec![
            ("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{team:team-1}:tokens".to_owned(), 15.0, Some(60)),
            ("{organization:org-1}:tokens".to_owned(), 15.0, Some(60)),
        ]
    );
}

#[test]
fn a_reserved_model_names_the_scopes_without_a_router_group() {
    let parts = SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        logging: model_scope_metadata(),
        slot: slot_from(reserved_parts("gpt-4", "reserved-group")),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    };
    assert_eq!(
        success_increments(&parts),
        vec![
            ("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60)),
            ("{team:team-1}:tokens".to_owned(), 15.0, Some(60)),
            ("{organization:org-1}:tokens".to_owned(), 15.0, Some(60)),
            (
                "{model_per_key:key-1:reserved-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_team:team-1:reserved-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_organization:org-1:reserved-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
            (
                "{model_per_project:proj-1:reserved-group}:tokens".to_owned(),
                15.0,
                Some(60)
            ),
        ]
    );
}
