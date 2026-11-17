#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;
use rust_decimal::Decimal;

use super::{AllowedModelRegion, NewCustomerRequest};
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::tests::{assert_decode, assert_validation, contract};

#[rstest]
#[case::five(r#"{"user_id": "u", "max_budget": 5, "budget_id": "b"}"#)]
#[case::zero(r#"{"user_id": "u", "max_budget": 0, "budget_id": "b"}"#)]
#[case::negative(r#"{"user_id": "u", "max_budget": -1, "budget_id": "b"}"#)]
fn rejects_a_set_max_budget_next_to_a_budget_id(#[case] text: &str) {
    let error = NewCustomerRequest::from_value(&contract(text)).expect_err("two budgets");
    assert_validation(error, "Set either 'max_budget' or 'budget_id', not both.");
}

#[rstest]
#[case::max_budget(r#"{"user_id": "u", "max_budget": 5}"#)]
#[case::budget_id(r#"{"user_id": "u", "budget_id": "b"}"#)]
#[case::null_max_budget(r#"{"user_id": "u", "max_budget": null, "budget_id": "b"}"#)]
#[case::missing_max_budget(r#"{"user_id": "u", "budget_id": "b", "max_budget": null}"#)]
fn accepts_one_way_to_set_the_budget(#[case] text: &str) {
    NewCustomerRequest::from_value(&contract(text)).expect("one budget source");
}

#[test]
fn keeps_every_default() {
    let request =
        NewCustomerRequest::from_value(&contract(r#"{"user_id": "u"}"#)).expect("only the user id");
    assert_eq!(request.user_id, "u");
    assert_eq!(request.alias, None);
    assert!(!request.blocked);
    assert_eq!(request.budget_id, None);
    assert_eq!(request.spend, None);
    assert_eq!(request.allowed_model_region, None);
    assert_eq!(request.default_model, None);
    assert_eq!(request.object_permission, None);
    assert_eq!(request.budget.max_budget, None);
    assert_eq!(request.budget.soft_budget, None);
    assert_eq!(request.budget.budget_duration, None);
    assert_eq!(request.budget.budget_reset_at, None);
    assert!(request.budget.model_max_budget.is_none());
}

#[test]
fn reads_every_declared_field() {
    let request = NewCustomerRequest::from_value(&contract(
        r#"{
            "user_id": "u",
            "alias": "a",
            "blocked": true,
            "spend": 1.5,
            "allowed_model_region": "eu",
            "default_model": "gpt-4o",
            "max_budget": 5,
            "soft_budget": 2,
            "tpd_limit": 7,
            "budget_duration": "1d",
            "model_max_budget": {"gpt-4o": {"max_budget": "1"}},
            "object_permission": {"models": ["gpt-4o"]}
        }"#,
    ))
    .expect("a full customer");
    assert_eq!(request.user_id, "u");
    assert_eq!(request.alias.as_deref(), Some("a"));
    assert!(request.blocked);
    assert_eq!(request.spend, Some(Amount(Decimal::new(15, 1))));
    assert_eq!(request.allowed_model_region, Some(AllowedModelRegion::Eu));
    assert_eq!(request.default_model.as_deref(), Some("gpt-4o"));
    assert_eq!(request.budget.max_budget, Some(Amount(Decimal::new(5, 0))));
    assert_eq!(request.budget.soft_budget, Some(Amount(Decimal::new(2, 0))));
    assert_eq!(request.budget.tpd_limit, Some(7));
    assert_eq!(request.budget.budget_duration.as_deref(), Some("1d"));
    assert!(request.budget.model_max_budget.is_some());
    assert!(request.object_permission.is_some());
}

#[test]
fn keeps_the_budget_id_on_the_customer() {
    let request =
        NewCustomerRequest::from_value(&contract(r#"{"user_id": "u", "budget_id": "b"}"#))
            .expect("a budget id");
    assert_eq!(request.budget_id.as_deref(), Some("b"));
    assert_eq!(request.budget.budget_id, None);
    assert_eq!(
        request.budget.supplied,
        BTreeSet::from(["budget_id".to_owned()])
    );
}

#[rstest]
#[case::text(r#"{"user_id": "u", "allowed_model_region": "mars"}"#)]
#[case::number(r#"{"user_id": 5}"#)]
fn rejects_a_value_of_the_wrong_type(#[case] text: &str) {
    let error = NewCustomerRequest::from_value(&contract(text)).expect_err("a typed field");
    assert_decode(error);
}

#[test]
fn requires_a_user_id() {
    let missing = NewCustomerRequest::from_value(&contract("{}")).expect_err("no user id");
    assert_decode(missing);
    let null = NewCustomerRequest::from_value(&contract(r#"{"user_id": null}"#))
        .expect_err("a null user id");
    assert_decode(null);
}
