#![cfg(test)]
use std::time::Duration;

use rstest::rstest;

use super::LegacyCarry;
use crate::model_budget_values::budget_config::BudgetConfig;
use crate::model_budget_values::budget_scope::BudgetScope;
use crate::model_budget_values::resolution::ResolvedModelBudget;
use crate::model_budget_values::tests::support::{amount, runtime};

fn resolved(budget_model: &str, budget_duration: &str) -> ResolvedModelBudget {
    ResolvedModelBudget {
        budget_model: budget_model.to_owned(),
        budget_config: BudgetConfig {
            max_budget: Some(amount(1, 0)),
            budget_duration: budget_duration.to_owned(),
            tpm_limit: None,
            rpm_limit: None,
        },
    }
}

fn carry(uptime: Duration) -> LegacyCarry<'static> {
    LegacyCarry {
        runtime: runtime(),
        uptime,
    }
}

#[rstest]
#[case::key(BudgetScope::Key, Some("virtual_key_spend:k1:openai/gpt-4o:1h"))]
#[case::end_user(BudgetScope::EndUser, Some("end_user_model_spend:k1:openai/gpt-4o:1h"))]
#[case::user(BudgetScope::User, None)]
#[case::team(BudgetScope::Team, None)]
fn legacy_carry_reads_the_old_key_for_key_and_end_user_scopes_only(
    #[case] scope: BudgetScope,
    #[case] expected: Option<&str>,
) {
    let key = carry(Duration::from_secs(10)).request_model_spend_cache_key(
        scope,
        "k1",
        "openai/gpt-4o",
        &resolved("gpt-4o", "1h"),
    );
    assert_eq!(key.as_deref(), expected);
}

#[rstest]
#[case::at_start(Duration::ZERO, true)]
#[case::just_inside_the_window(Duration::new(3_599, 999_999_999), true)]
#[case::exactly_one_window(Duration::from_secs(3_600), false)]
#[case::past_the_window(Duration::from_secs(7_200), false)]
fn legacy_carry_stops_after_one_budget_window(#[case] uptime: Duration, #[case] carried: bool) {
    let key = carry(uptime).request_model_spend_cache_key(
        BudgetScope::Key,
        "k1",
        "openai/gpt-4o",
        &resolved("gpt-4o", "1h"),
    );
    assert_eq!(key.is_some(), carried);
}

#[test]
fn legacy_carry_window_follows_the_budget_duration() {
    let uptime = Duration::from_secs(3_600);
    let long_window = carry(uptime).request_model_spend_cache_key(
        BudgetScope::Key,
        "k1",
        "openai/gpt-4o",
        &resolved("gpt-4o", "2h"),
    );
    assert!(long_window.is_some());
}

#[test]
fn legacy_carry_skips_a_key_equal_to_the_current_one() {
    let key = carry(Duration::ZERO).request_model_spend_cache_key(
        BudgetScope::Key,
        "k1",
        "gpt-4o",
        &resolved("gpt-4o", "1h"),
    );
    assert_eq!(key, None);
}

#[test]
fn legacy_carry_uses_the_request_model_not_the_configured_name() {
    let key = carry(Duration::ZERO)
        .request_model_spend_cache_key(
            BudgetScope::Key,
            "k1",
            "gpt-4o-mini",
            &resolved("gpt-4o", "1d"),
        )
        .unwrap();
    assert!(key.ends_with(":gpt-4o-mini:1d"));
}

#[test]
fn legacy_carry_gives_up_on_a_duration_the_parser_rejects() {
    let key = carry(Duration::ZERO).request_model_spend_cache_key(
        BudgetScope::Key,
        "k1",
        "openai/gpt-4o",
        &resolved("gpt-4o", "soon"),
    );
    assert_eq!(key, None);
}

#[test]
fn legacy_carry_treats_a_negative_window_as_already_over() {
    let key = carry(Duration::ZERO).request_model_spend_cache_key(
        BudgetScope::Key,
        "k1",
        "openai/gpt-4o",
        &resolved("gpt-4o", "-5s"),
    );
    assert_eq!(key, None);
}
