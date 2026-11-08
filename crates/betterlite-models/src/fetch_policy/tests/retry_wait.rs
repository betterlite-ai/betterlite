use rstest::rstest;

use super::support;
use crate::fetch_policy::retry::{MAX_FETCH_ATTEMPTS, MAX_FETCH_WAIT_SECONDS, retry_wait_seconds};

#[rstest]
#[case::retry_after_used(Some("3.5"), 1, &[], "3.5")]
#[case::retry_after_capped(Some("45"), 1, &[], "30")]
#[case::retry_after_at_cap(Some("30"), 1, &[], "30")]
#[case::first_backoff(None, 0, &["0.25"], "1.25")]
#[case::second_backoff(None, 1, &["0.25"], "2.25")]
#[case::third_backoff(None, 2, &["0.5"], "4.5")]
#[case::capped_backoff(None, 5, &["0.25"], "30")]
#[case::overflowing_backoff(None, 64, &["0.25"], "30")]
#[case::overflowing_jitter(None, 1, &["79228162514264337593543950335"], "30")]
fn catalog_fetch_policy_behavior_tests_behavior(
    #[case] retry_after: Option<&str>,
    #[case] attempt: u32,
    #[case] jitter: &[&str],
    #[case] expected: &str,
) {
    let mut source = support::ScriptedJitter::new(jitter);
    assert_eq!(
        retry_wait_seconds(&support::retryable(retry_after), attempt, &mut source),
        support::decimal(expected)
    );
    assert_eq!(MAX_FETCH_WAIT_SECONDS, support::decimal("30"));
    assert_eq!(MAX_FETCH_ATTEMPTS, 3);
}
