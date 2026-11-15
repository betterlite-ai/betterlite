#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::reconciliation_operations::tests::support::{
    SuccessParts, SuccessResponseFixture, logging_metadata, metadata, success_increments,
    success_ops, usage,
};
use crate::request_stash::RequestStashSlot;
use crate::usage_resolution::ResponseObject;

fn parts(entries: &[(&str, Json)]) -> SuccessParts {
    SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        logging: logging_metadata(),
        metadata: metadata(entries),
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        request_user: None,
        model_group: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    }
}

fn charged() -> Vec<(String, f64, Option<i64>)> {
    vec![("{api_key:key-1}:tokens".to_owned(), 15.0_f64, Some(60))]
}

#[test]
fn an_absent_origin_emits_operations() {
    assert_eq!(success_increments(&parts(&[])), charged());
}

#[test]
fn a_null_origin_emits_operations() {
    assert_eq!(
        success_increments(&parts(&[("internal_call_origin", Json::Null)])),
        charged()
    );
}

#[test]
fn an_empty_origin_emits_operations() {
    assert_eq!(
        success_increments(&parts(&[(
            "internal_call_origin",
            Json::Text(String::new())
        )])),
        charged()
    );
}

#[test]
fn the_compaction_origin_emits_operations() {
    assert_eq!(
        success_increments(&parts(&[(
            "internal_call_origin",
            Json::Text("autorouter_compaction".to_owned())
        )])),
        charged()
    );
}

#[test]
fn a_foreign_origin_exempts_the_request() {
    assert!(
        success_increments(&parts(&[(
            "internal_call_origin",
            Json::Text("background_evaluation".to_owned())
        )]))
        .is_empty()
    );
}

#[test]
fn a_non_text_origin_is_rejected() {
    let rejected = parts(&[("internal_call_origin", Json::Number(Number::from(5_i64)))]);
    let error = success_ops(&rejected).expect_err("a non-text origin is rejected");
    assert!(error.to_string().contains("internal_call_origin"));
}
