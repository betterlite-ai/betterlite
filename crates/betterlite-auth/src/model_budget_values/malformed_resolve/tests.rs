#![cfg(test)]
use rstest::rstest;

use super::resolve_model_budget;
use crate::model_budget_values::tests::support::{Warnings, amount, budgets, catalog, runtime};

fn resolved_name(model: &str, configured: &str) -> Option<String> {
    let budgets = budgets(configured);
    resolve_model_budget(model, &budgets, &catalog(), runtime())
        .map(|resolved| resolved.budget_model)
}

#[rstest]
#[case::exact_name(
    "gpt-4",
    r#"{"gpt-4": {"max_budget": 1, "budget_duration": "1d"}}"#,
    Some("gpt-4")
)]
#[case::provider_prefix_stripped(
    "openai/gpt-4o",
    r#"{"gpt-4o": {"max_budget": 1, "budget_duration": "1d"}}"#,
    Some("gpt-4o")
)]
#[case::requested_name_beats_bare_name(
    "openai/gpt-4o",
    r#"{"gpt-4o": {"max_budget": 1, "budget_duration": "1d"}, "openai/gpt-4o": {"max_budget": 2, "budget_duration": "1d"}}"#,
    Some("openai/gpt-4o")
)]
#[case::bedrock_base(
    "us.anthropic.claude-opus-4-8",
    r#"{"anthropic.claude-opus-4-8": {"max_budget": 1, "budget_duration": "1d"}}"#,
    Some("anthropic.claude-opus-4-8")
)]
#[case::bedrock_family(
    "us.anthropic.claude-opus-4-8",
    r#"{"claude-opus-4-8": {"max_budget": 1, "budget_duration": "1d"}}"#,
    Some("claude-opus-4-8")
)]
#[case::bedrock_base_beats_family(
    "us.anthropic.claude-opus-4-8",
    r#"{"claude-opus-4-8": {"max_budget": 1, "budget_duration": "1d"}, "anthropic.claude-opus-4-8": {"max_budget": 1, "budget_duration": "1d"}}"#,
    Some("anthropic.claude-opus-4-8")
)]
#[case::no_candidate_configured(
    "gpt-4",
    r#"{"claude": {"max_budget": 1, "budget_duration": "1d"}}"#,
    None
)]
#[case::no_budgets_at_all("gpt-4", "{}", None)]
#[case::null_entry_is_absent(
    "openai/gpt-4o",
    r#"{"openai/gpt-4o": null, "gpt-4o": {"max_budget": 1, "budget_duration": "1d"}}"#,
    Some("gpt-4o")
)]
fn malformed_resolve_picks_the_first_configured_candidate(
    #[case] model: &str,
    #[case] configured: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(resolved_name(model, configured).as_deref(), expected);
}

#[test]
fn malformed_resolve_returns_the_configured_name_and_its_config() {
    let configured =
        budgets(r#"{"gpt-4o": {"max_budget": 3, "budget_duration": "1h", "tpm_limit": 9}}"#);
    let resolved =
        resolve_model_budget("openai/gpt-4o", &configured, &catalog(), runtime()).unwrap();
    assert_eq!(resolved.budget_model, "gpt-4o");
    assert_eq!(resolved.budget_config.max_budget, Some(amount(3, 0)));
    assert_eq!(resolved.budget_config.budget_duration, "1h");
    assert_eq!(resolved.budget_config.tpm_limit, Some(9));
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
