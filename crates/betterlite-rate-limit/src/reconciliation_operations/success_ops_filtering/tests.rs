#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;

use crate::reconciliation_operations::tests::support::{
    SuccessParts, SuccessResponseFixture, key_owned_metadata, model_scope_metadata,
    success_increments, usage,
};
use crate::request_stash::RequestStashSlot;
use crate::usage_resolution::ResponseObject;

fn parts_with(metadata: MetadataMap) -> SuccessParts {
    SuccessParts {
        response: SuccessResponseFixture::Modelled(ResponseObject::Usage(usage(10, 5))),
        logging: model_scope_metadata(),
        model_group: Some("gpt-4".to_owned()),
        metadata,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        request_user: None,
        slot: RequestStashSlot::default(),
        call_id: None,
        groups: BTreeMap::new(),
        window_size: 60,
    }
}

fn has(increments: &[(String, f64, Option<i64>)], wanted: &str) -> bool {
    increments.iter().any(|(key, _, _)| key.as_str() == wanted)
}

#[test]
fn a_key_owned_model_limit_drops_the_team_model_scope() {
    let increments = success_increments(&parts_with(key_owned_metadata("gpt-4")));
    assert_eq!(increments.len(), 6);
    assert!(!has(&increments, "{model_per_team:team-1:gpt-4}:tokens"));
    assert!(has(&increments, "{model_per_key:key-1:gpt-4}:tokens"));
}

#[test]
fn a_key_without_its_own_model_limit_keeps_the_team_model_scope() {
    let increments = success_increments(&parts_with(MetadataMap::new()));
    assert_eq!(increments.len(), 7);
    assert!(has(&increments, "{model_per_team:team-1:gpt-4}:tokens"));
}

#[test]
fn an_owned_limit_for_another_model_keeps_the_team_model_scope() {
    let increments = success_increments(&parts_with(key_owned_metadata("other-model")));
    assert_eq!(increments.len(), 7);
    assert!(has(&increments, "{model_per_team:team-1:gpt-4}:tokens"));
}
