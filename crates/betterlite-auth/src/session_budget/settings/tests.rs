#![cfg(test)]

use std::time::Duration;

use betterlite_env::EnvError;

use super::DEFAULT_TTL_SECONDS;
use super::SessionBudgetTtlError;
use super::TTL_VARIABLE;
use super::session_budget_ttl;

#[test]
fn ttl_default_constant_is_3600() {
    assert_eq!(DEFAULT_TTL_SECONDS, 3600);
    assert_eq!(TTL_VARIABLE, "LITELLM_MAX_BUDGET_PER_SESSION_TTL");
}

#[test]
fn ttl_defaults_to_3600_seconds_when_the_variable_is_unset() {
    assert_eq!(session_budget_ttl(None).unwrap(), Duration::from_secs(3600));
}

#[test]
fn ttl_reads_a_plain_number() {
    assert_eq!(
        session_budget_ttl(Some("120")).unwrap(),
        Duration::from_secs(120)
    );
}

#[test]
fn ttl_trims_spaces_around_the_number() {
    assert_eq!(
        session_budget_ttl(Some(" 240 ")).unwrap(),
        Duration::from_secs(240)
    );
}

#[test]
fn ttl_reads_a_leading_plus() {
    assert_eq!(
        session_budget_ttl(Some("+60")).unwrap(),
        Duration::from_secs(60)
    );
}

#[test]
fn ttl_reads_zero_as_a_zero_second_lifetime() {
    assert_eq!(
        session_budget_ttl(Some("0")).unwrap(),
        Duration::from_secs(0)
    );
}

#[test]
fn ttl_empty_text_is_the_default() {
    assert_eq!(
        session_budget_ttl(Some("")).unwrap(),
        Duration::from_secs(DEFAULT_TTL_SECONDS)
    );
}

fn number_error() -> SessionBudgetTtlError {
    SessionBudgetTtlError::Env(EnvError::Number {
        name: TTL_VARIABLE.to_owned(),
    })
}

#[test]
fn ttl_rejects_text_that_is_not_a_number() {
    assert_eq!(
        session_budget_ttl(Some("soon")).unwrap_err(),
        number_error()
    );
}

#[test]
fn ttl_rejects_a_fraction() {
    assert_eq!(session_budget_ttl(Some("1.5")).unwrap_err(), number_error());
}

#[test]
fn ttl_rejects_a_unit_suffix() {
    assert_eq!(session_budget_ttl(Some("2h")).unwrap_err(), number_error());
}

#[test]
fn ttl_rejects_a_negative_number() {
    assert_eq!(
        session_budget_ttl(Some("-5")).unwrap_err(),
        SessionBudgetTtlError::Negative
    );
}
