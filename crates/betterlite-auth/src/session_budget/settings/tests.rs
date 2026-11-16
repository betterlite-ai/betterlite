use super::DEFAULT_TTL_SECONDS;
use super::SessionBudgetTtlError;
use super::TTL_VARIABLE;
use super::session_budget_ttl;
use betterlite_env::EnvError;
use rstest::rstest;
use std::time::Duration;

#[test]
fn ttl_default_constant_is_3600() {
    assert_eq!(DEFAULT_TTL_SECONDS, 3600);
    assert_eq!(TTL_VARIABLE, "LITELLM_MAX_BUDGET_PER_SESSION_TTL");
}

#[test]
fn ttl_defaults_to_3600_seconds_when_the_variable_is_unset() {
    assert_eq!(session_budget_ttl(None).unwrap(), Duration::from_secs(3600));
}

#[rstest]
#[case::plain("120", 120)]
#[case::surrounding_spaces(" 240 ", 240)]
#[case::leading_plus("+60", 60)]
#[case::zero("0", 0)]
fn ttl_reads_the_configured_number(#[case] text: &str, #[case] seconds: u64) {
    assert_eq!(
        session_budget_ttl(Some(text)).unwrap(),
        Duration::from_secs(seconds)
    );
}

#[test]
fn ttl_empty_text_is_the_default() {
    assert_eq!(
        session_budget_ttl(Some("")).unwrap(),
        Duration::from_secs(DEFAULT_TTL_SECONDS)
    );
}

#[rstest]
#[case::text("soon")]
#[case::fraction("1.5")]
#[case::unit("2h")]
fn ttl_rejects_text_that_is_not_a_whole_number(#[case] text: &str) {
    let error = session_budget_ttl(Some(text)).unwrap_err();
    assert_eq!(
        error,
        SessionBudgetTtlError::Env(EnvError::Number {
            name: TTL_VARIABLE.to_owned(),
        })
    );
}

#[test]
fn ttl_rejects_a_negative_number() {
    assert_eq!(
        session_budget_ttl(Some("-5")).unwrap_err(),
        SessionBudgetTtlError::Negative
    );
}
