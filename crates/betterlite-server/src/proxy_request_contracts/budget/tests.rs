#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;
use rust_decimal::Decimal;

use super::{BudgetDeleteRequest, BudgetNewRequest, BudgetRequest};
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::tests::{assert_decode, contract};

fn supplied(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn keeps_every_field_optional() {
    let request = BudgetNewRequest::from_value(&contract("{}")).expect("an empty budget");
    assert_eq!(request.budget_id, None);
    assert_eq!(request.max_budget, None);
    assert_eq!(request.soft_budget, None);
    assert_eq!(request.max_parallel_requests, None);
    assert_eq!(request.tpm_limit, None);
    assert_eq!(request.rpm_limit, None);
    assert_eq!(request.tpd_limit, None);
    assert_eq!(request.budget_duration, None);
    assert_eq!(request.model_max_budget, None);
    assert_eq!(request.budget_reset_at, None);
    assert!(request.supplied.is_empty());
}

#[test]
fn decodes_every_declared_type() {
    let request = BudgetNewRequest::from_value(&contract(
        r#"{
            "budget_id": "b",
            "max_budget": 5,
            "soft_budget": 5.5,
            "max_parallel_requests": 1,
            "tpm_limit": 2,
            "rpm_limit": 3,
            "tpd_limit": 4,
            "budget_duration": "1d",
            "model_max_budget": {"gpt-4o": {"max_budget": "1"}},
            "budget_reset_at": "2024-06-01T00:00:00Z"
        }"#,
    ))
    .expect("a full budget");
    assert_eq!(request.budget_id.as_deref(), Some("b"));
    assert_eq!(request.max_budget, Some(Amount(Decimal::new(5, 0))));
    assert_eq!(request.soft_budget, Some(Amount(Decimal::new(55, 1))));
    assert_eq!(request.max_parallel_requests, Some(1));
    assert_eq!(request.tpm_limit, Some(2));
    assert_eq!(request.rpm_limit, Some(3));
    assert_eq!(request.tpd_limit, Some(4));
    assert_eq!(request.budget_duration.as_deref(), Some("1d"));
    assert!(request.model_max_budget.is_some());
    let reset = request.budget_reset_at.expect("a reset time");
    assert_eq!(
        reset.utc(),
        "2024-06-01T00:00:00Z".parse().expect("a fixed time")
    );
}

#[test]
fn records_which_fields_were_sent() {
    let request = BudgetNewRequest::from_value(&contract(
        r#"{"budget_id": "b", "max_budget": null, "tpd_limit": 5}"#,
    ))
    .expect("a partial budget");
    assert_eq!(
        request.supplied,
        supplied(&["budget_id", "max_budget", "tpd_limit"])
    );
    assert_eq!(request.max_budget, None);
    assert_eq!(request.tpd_limit, Some(5));
}

#[rstest]
#[case::boolean(r#"{"max_budget": true}"#)]
#[case::text(r#"{"soft_budget": "lots"}"#)]
#[case::fraction(r#"{"tpm_limit": 1.5}"#)]
#[case::bad_time(r#"{"budget_reset_at": "yesterday"}"#)]
fn rejects_a_value_of_the_wrong_type(#[case] text: &str) {
    let error = BudgetNewRequest::from_value(&contract(text)).expect_err("a typed field");
    assert_decode(error);
}

#[test]
fn requires_the_budget_list() {
    let missing = BudgetRequest::from_value(&contract("{}")).expect_err("no budgets");
    assert_decode(missing);
    let null = BudgetRequest::from_value(&contract(r#"{"budgets": null}"#)).expect_err("a null");
    assert_decode(null);
    let empty = BudgetRequest::from_value(&contract(r#"{"budgets": []}"#)).expect("no budgets");
    assert!(empty.budgets.is_empty());
}

#[test]
fn requires_the_delete_id() {
    let missing = BudgetDeleteRequest::from_value(&contract("{}")).expect_err("no id");
    assert_decode(missing);
    let wrong = BudgetDeleteRequest::from_value(&contract(r#"{"id": 5}"#)).expect_err("a number");
    assert_decode(wrong);
    let request = BudgetDeleteRequest::from_value(&contract(r#"{"id": "b"}"#)).expect("an id");
    assert_eq!(request.id, "b");
}
