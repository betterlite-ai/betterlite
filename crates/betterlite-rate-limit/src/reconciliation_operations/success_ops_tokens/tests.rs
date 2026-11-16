#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;

use crate::reconciliation_operations::tests::support::{
    SuccessParts, SuccessResponseFixture, logging_metadata, success_increments, total_only, usage,
};
use crate::request_stash::RequestStashSlot;
use crate::usage_resolution::ResponseObject;

fn modelled(response: ResponseObject) -> SuccessParts {
    SuccessParts {
        response: SuccessResponseFixture::Modelled(response),
        logging: logging_metadata(),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    }
}

fn charged(tokens: f64) -> Vec<(String, f64, Option<i64>)> {
    vec![("{api_key:key-1}:tokens".to_owned(), tokens, Some(60))]
}

#[test]
fn the_total_rate_limit_type_reads_total_tokens() {
    assert_eq!(
        success_increments(&modelled(ResponseObject::Usage(usage(100, 50)))),
        charged(150.0)
    );
}

#[test]
fn the_output_rate_limit_type_reads_completion_tokens() {
    let parts = SuccessParts {
        rate_limit_type: TokenRateLimitType::Output,
        ..modelled(ResponseObject::Usage(usage(100, 50)))
    };
    assert_eq!(success_increments(&parts), charged(50.0));
}

#[test]
fn the_input_rate_limit_type_reads_prompt_tokens() {
    let parts = SuccessParts {
        rate_limit_type: TokenRateLimitType::Input,
        ..modelled(ResponseObject::Usage(usage(100, 50)))
    };
    assert_eq!(success_increments(&parts), charged(100.0));
}

#[test]
fn an_aggregate_only_usage_charges_its_total() {
    let parts = SuccessParts {
        rate_limit_type: TokenRateLimitType::Output,
        ..modelled(ResponseObject::Usage(total_only(120)))
    };
    assert_eq!(success_increments(&parts), charged(120.0));
}

#[test]
fn a_modelled_response_wins_over_combined_usage() {
    let parts = SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(20, 10))),
        combined_usage: Some(usage(999, 999)),
        logging: logging_metadata(),
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    };
    assert_eq!(success_increments(&parts), charged(30.0));
}

#[test]
fn a_modelled_response_without_usage_ignores_combined_usage() {
    let parts = SuccessParts {
        combined_usage: Some(usage(40, 20)),
        logging: logging_metadata(),
        response: SuccessResponseFixture::Modelled(ResponseObject::WithUsage(None)),
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    };
    assert!(success_increments(&parts).is_empty());
}

#[test]
fn an_unmodelled_response_charges_combined_usage() {
    let parts = SuccessParts {
        response: SuccessResponseFixture::Unmodelled,
        combined_usage: Some(usage(10, 5)),
        logging: logging_metadata(),
        rate_limit_type: TokenRateLimitType::Total,
        metadata: MetadataMap::new(),
        request_user: None,
        model_group: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    };
    assert_eq!(success_increments(&parts), charged(15.0));
}

#[test]
fn an_unmodelled_response_without_combined_usage_builds_nothing() {
    let parts = SuccessParts {
        response: SuccessResponseFixture::Unmodelled,
        logging: logging_metadata(),
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
    assert!(success_increments(&parts).is_empty());
}
