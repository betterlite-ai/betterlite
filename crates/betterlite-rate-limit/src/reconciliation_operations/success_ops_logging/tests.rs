#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;

use crate::reconciliation_operations::tests::support::{
    LogCapture, SuccessParts, SuccessResponseFixture, logging_metadata, scopes, slot_from,
    success_increments, usage,
};
use crate::request_stash::StashState;
use crate::usage_resolution::ResponseObject;

const RELEASE_MESSAGE: &str = "Releasing unused TPM budget on success";

fn parts(reserved_tokens: i64) -> SuccessParts {
    SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        logging: logging_metadata(),
        slot: slot_from(StashState {
            reserved_tokens,
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
        }),
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
fn releasing_unused_budget_is_logged() {
    let capture = LogCapture::start();
    assert_eq!(
        success_increments(&parts(100)),
        vec![("{api_key:key-1}:tokens".to_owned(), -85.0_f64, Some(60))]
    );
    let text = capture.text();
    assert!(text.contains(RELEASE_MESSAGE));
    assert!(text.contains("reserved=100"));
    assert!(text.contains("actual=15"));
    assert!(text.contains("release=85"));
}

#[test]
fn a_charge_above_the_reservation_is_not_logged() {
    let capture = LogCapture::start();
    assert_eq!(
        success_increments(&parts(10)),
        vec![("{api_key:key-1}:tokens".to_owned(), 5.0_f64, Some(60))]
    );
    assert!(!capture.text().contains(RELEASE_MESSAGE));
}

#[test]
fn an_exact_reservation_is_not_logged() {
    let capture = LogCapture::start();
    assert!(success_increments(&parts(15)).is_empty());
    assert!(!capture.text().contains(RELEASE_MESSAGE));
}
