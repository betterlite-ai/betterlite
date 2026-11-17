#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{UserInfoResponse, UserInfoV2Response};
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::tests::{assert_decode, contract};

#[rstest]
#[case::user_id(r#"{"user_info": null, "keys": [], "teams": []}"#)]
#[case::user_info(r#"{"user_id": "u", "keys": [], "teams": []}"#)]
#[case::keys(r#"{"user_id": "u", "user_info": null, "teams": []}"#)]
#[case::teams(r#"{"user_id": "u", "user_info": null, "keys": []}"#)]
#[case::null_keys(r#"{"user_id": "u", "user_info": null, "keys": null, "teams": []}"#)]
#[case::null_teams(r#"{"user_id": "u", "user_info": null, "keys": [], "teams": null}"#)]
fn user_info_requires_every_declared_key(#[case] text: &str) {
    let error = UserInfoResponse::from_value(&contract(text)).expect_err("a required key");
    assert_decode(error);
}

#[test]
fn user_info_allows_null_ids_and_keeps_the_lists() {
    let response = UserInfoResponse::from_value(&contract(
        r#"{"user_id": null, "user_info": null, "keys": [1, "x"], "teams": [{"id": "t1"}]}"#,
    ))
    .expect("a user info response");
    assert_eq!(response.user_id, None);
    assert_eq!(response.user_info, None);
    assert_eq!(response.keys, vec![contract("1"), contract(r#""x""#)]);
    assert_eq!(response.teams, vec![contract(r#"{"id": "t1"}"#)]);
}

#[test]
fn user_info_reads_the_user_map() {
    let response = UserInfoResponse::from_value(&contract(
        r#"{"user_id": "u", "user_info": {"a": 1}, "keys": [], "teams": []}"#,
    ))
    .expect("a user info response");
    assert_eq!(response.user_id.as_deref(), Some("u"));
    assert!(response.user_info.is_some());
}

#[test]
fn user_info_v2_defaults_to_zero_and_empty() {
    let response =
        UserInfoV2Response::from_value(&contract(r#"{"user_id": "u"}"#)).expect("the user id");
    assert_eq!(response.user_email, None);
    assert_eq!(response.user_alias, None);
    assert_eq!(response.user_role, None);
    assert_eq!(response.spend, Amount(Decimal::ZERO));
    assert_eq!(response.max_budget, None);
    assert!(response.models.is_empty());
    assert_eq!(response.budget_duration, None);
    assert_eq!(response.budget_reset_at, None);
    assert_eq!(response.metadata, None);
    assert_eq!(response.created_at, None);
    assert_eq!(response.updated_at, None);
    assert_eq!(response.sso_user_id, None);
    assert!(response.teams.is_empty());
    assert_eq!(response.object_permission, None);
    assert_eq!(response.model_max_budget, None);
    assert_eq!(response.model_max_budget_usage, None);
}

#[test]
fn user_info_v2_reads_every_field() {
    let response = UserInfoV2Response::from_value(&contract(
        r#"{
            "user_id": "u",
            "user_email": "a@example.com",
            "user_alias": "a",
            "user_role": "internal_user",
            "spend": 1.5,
            "max_budget": 10,
            "models": ["gpt-4o"],
            "budget_duration": "1d",
            "budget_reset_at": "2024-06-01T00:00:00Z",
            "metadata": {"a": 1},
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-02-01T00:00:00Z",
            "sso_user_id": "s",
            "teams": ["t"],
            "object_permission": {"object_permission_id": "op", "models": ["gpt-4o"]},
            "model_max_budget": {"gpt-4o": {"max_budget": "1"}},
            "model_max_budget_usage": {"gpt-4o": {"spend": 0.5}}
        }"#,
    ))
    .expect("a full user");
    assert_eq!(response.user_id, "u");
    assert_eq!(response.user_email.as_deref(), Some("a@example.com"));
    assert_eq!(response.user_alias.as_deref(), Some("a"));
    assert_eq!(response.user_role.as_deref(), Some("internal_user"));
    assert_eq!(response.spend, Amount(Decimal::new(15, 1)));
    assert_eq!(response.max_budget, Some(Amount(Decimal::new(10, 0))));
    assert_eq!(response.models, vec!["gpt-4o".to_owned()]);
    assert_eq!(response.budget_duration.as_deref(), Some("1d"));
    let reset = response.budget_reset_at.expect("a reset time");
    assert_eq!(
        reset.utc(),
        "2024-06-01T00:00:00Z".parse().expect("a fixed time")
    );
    assert!(response.metadata.is_some());
    assert!(response.created_at.is_some());
    assert!(response.updated_at.is_some());
    assert_eq!(response.sso_user_id.as_deref(), Some("s"));
    assert_eq!(response.teams, vec!["t".to_owned()]);
    let permission = response.object_permission.expect("an object permission");
    assert_eq!(permission.object_permission_id, "op");
    assert!(response.model_max_budget.is_some());
    let usage = response.model_max_budget_usage.expect("a usage map");
    assert!(usage.contains_key("gpt-4o"));
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"user_id": null}"#)]
#[case::number(r#"{"user_id": 5}"#)]
fn user_info_v2_requires_a_user_id(#[case] text: &str) {
    let error = UserInfoV2Response::from_value(&contract(text)).expect_err("a user id");
    assert_decode(error);
}

#[rstest]
#[case::spend(r#"{"user_id": "u", "spend": "lots"}"#)]
#[case::models(r#"{"user_id": "u", "models": "gpt-4o"}"#)]
#[case::created_at(r#"{"user_id": "u", "created_at": "yesterday"}"#)]
#[case::metadata(r#"{"user_id": "u", "metadata": [1]}"#)]
fn user_info_v2_rejects_a_value_of_the_wrong_type(#[case] text: &str) {
    let error = UserInfoV2Response::from_value(&contract(text)).expect_err("a typed field");
    assert_decode(error);
}
