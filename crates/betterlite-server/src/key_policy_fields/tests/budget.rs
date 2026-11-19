#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::object;
use crate::key_policy_fields::model_budget::{EXAMPLE_URL, validate_model_max_budget};

#[test]
fn model_budget_empty() {
    validate_model_max_budget(None).unwrap();
    let empty = BTreeMap::<String, JsonData>::new();
    validate_model_max_budget(Some(&empty)).unwrap();
}

#[test]
fn model_budget_numeric_aliases() {
    let by_limit = object(
        r#"{"gpt-4":{"budget_limit":"2.5","time_period":"1d","rpm_limit":10,"tpm_limit":20}}"#,
    );
    validate_model_max_budget(Some(&by_limit)).unwrap();
    // a JSON number is a budget_limit too, since LiteLLM hands the value to float()
    let numeric_limit = object(r#"{"gpt-4":{"budget_limit":2.5}}"#);
    validate_model_max_budget(Some(&numeric_limit)).unwrap();
    let by_budget = object(r#"{"gpt-4":{"max_budget":10,"budget_duration":"1d"}}"#);
    validate_model_max_budget(Some(&by_budget)).unwrap();
    // nulls take the model defaults, and unknown keys are not BudgetConfig fields
    let sparse = object(r#"{"gpt-4":{"max_budget":null,"tpm_limit":null,"unknown":"value"}}"#);
    validate_model_max_budget(Some(&sparse)).unwrap();
    // the alias replaces the field it names, so a bad max_budget beside budget_limit is unused
    let aliased = object(r#"{"gpt-4":{"budget_limit":"3","max_budget":"not a number"}}"#);
    validate_model_max_budget(Some(&aliased)).unwrap();
    let timed = object(r#"{"gpt-4":{"time_period":null,"budget_duration":5}}"#);
    validate_model_max_budget(Some(&timed)).unwrap();
}

#[rstest]
#[case::budget_limit_text(r#"{"gpt-4":{"budget_limit":"cheap"}}"#)]
#[case::budget_limit_null(r#"{"gpt-4":{"budget_limit":null}}"#)]
#[case::max_budget_text(r#"{"gpt-4":{"max_budget":"10"}}"#)]
#[case::max_budget_list(r#"{"gpt-4":{"max_budget":[1]}}"#)]
#[case::budget_duration_number(r#"{"gpt-4":{"budget_duration":5}}"#)]
#[case::time_period_bool(r#"{"gpt-4":{"time_period":true}}"#)]
#[case::tpm_limit_fraction(r#"{"gpt-4":{"tpm_limit":1.5}}"#)]
#[case::rpm_limit_text(r#"{"gpt-4":{"rpm_limit":"ten"}}"#)]
#[case::entry_not_object(r#"{"gpt-4":"cheap"}"#)]
fn model_budget_invalid_values(#[case] document: &str) {
    let error = validate_model_max_budget(Some(&object(document))).unwrap_err();
    let text = error.to_string();
    assert!(text.starts_with("Invalid model_max_budget: "), "{text}");
    assert!(text.contains("Example of valid model_max_budget"), "{text}");
    assert!(text.contains(EXAMPLE_URL), "{text}");
}

#[test]
fn model_budget_without_license_is_allowed() {
    // LiteLLM answers "You must have an enterprise license" here; betterlite validates and stores it
    let budgets = object(r#"{"gpt-4":{"max_budget":10.5,"budget_duration":"30d"}}"#);
    validate_model_max_budget(Some(&budgets)).unwrap();
}
