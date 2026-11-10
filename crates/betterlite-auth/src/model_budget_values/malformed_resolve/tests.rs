#![cfg(test)]
use rstest::rstest;

use crate::model_budget_values::resolution::resolve_model_budget;
use crate::model_budget_values::tests::support::{Warnings, budgets, catalog, runtime};

fn resolved_name(model: &str, configured: &str) -> Option<String> {
    let budgets = budgets(configured);
    resolve_model_budget(model, &budgets, &catalog(), runtime())
        .map(|resolved| resolved.budget_model)
}

#[rstest]
#[case::missing_duration(r#"{"max_budget": 1}"#)]
#[case::bad_duration(r#"{"max_budget": 1, "budget_duration": "soon"}"#)]
#[case::bad_budget(r#"{"max_budget": "x", "budget_duration": "1d"}"#)]
#[case::not_an_object("5")]
fn malformed_resolve_warns_and_falls_through_to_the_next_candidate(#[case] bad_entry: &str) {
    let warnings = Warnings::capture();
    let configured = format!(
        r#"{{"openai/gpt-4o": {bad_entry}, "gpt-4o": {{"max_budget": 1, "budget_duration": "1d"}}}}"#
    );
    assert_eq!(
        resolved_name("openai/gpt-4o", &configured).as_deref(),
        Some("gpt-4o")
    );
    assert_eq!(warnings.count(), 1);
}

#[test]
fn malformed_resolve_warns_once_per_bad_candidate_and_ends_with_none() {
    let warnings = Warnings::capture();
    let configured = r#"{"openai/gpt-4o": {"max_budget": 1}, "gpt-4o": "x"}"#;
    assert_eq!(resolved_name("openai/gpt-4o", configured), None);
    assert_eq!(warnings.count(), 2);
}

#[test]
fn malformed_resolve_does_not_warn_for_absent_or_null_entries() {
    let warnings = Warnings::capture();
    let configured = r#"{"openai/gpt-4o": null, "other": {"max_budget": 1}}"#;
    assert_eq!(resolved_name("openai/gpt-4o", configured), None);
    assert_eq!(warnings.count(), 0);
}

#[test]
fn malformed_resolve_does_not_warn_when_the_first_candidate_is_usable() {
    let warnings = Warnings::capture();
    let configured = r#"{"gpt-4": {"max_budget": 1, "budget_duration": "1d"}}"#;
    assert_eq!(resolved_name("gpt-4", configured).as_deref(), Some("gpt-4"));
    assert_eq!(warnings.count(), 0);
}
