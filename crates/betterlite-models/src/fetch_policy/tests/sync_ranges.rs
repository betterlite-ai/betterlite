use rstest::rstest;
use rust_decimal::Decimal;

use super::support;
use crate::fetch_policy::retry::fetch_with_retry_sync;

#[derive(Debug)]
enum Outcome {
    Reloaded,
    Exhausted(&'static str),
    Failed,
}

#[rstest]
#[case::exhaust_three(1, 4, 3, &["2.25", "4.5"], Outcome::Exhausted("(after 3 attempts)"))]
#[case::offset_range(0, 3, 3, &["1.25", "2.5"], Outcome::Exhausted("(after 2 attempts)"))]
#[case::empty_range(1, 1, 0, &[], Outcome::Failed)]
#[case::success(1, 4, 0, &[], Outcome::Reloaded)]
fn catalog_fetch_policy_behavior_tests_behavior(
    #[case] start: u32,
    #[case] end: u32,
    #[case] retries: usize,
    #[case] expected_sleeps: &[&str],
    #[case] outcome: Outcome,
) {
    let mut replies = Vec::new();
    if retries > 0 {
        for _ in 0..retries {
            replies.push(Ok(support::response(503, "{}")));
        }
    }
    if retries == 0 && matches!(outcome, Outcome::Reloaded) {
        replies.push(Ok(support::response(200, "{}")));
    }
    let expected_calls = match outcome {
        Outcome::Reloaded => 1,
        Outcome::Exhausted(_) => retries,
        Outcome::Failed => 0,
    };
    let client = support::ScriptedClient::new(replies);
    let mut sleeps = Vec::new();
    let mut record = |seconds: Decimal| sleeps.push(seconds);
    let mut jitter = support::ScriptedJitter::new(&["0.25", "0.5"]);
    let result = fetch_with_retry_sync(
        &support::request(),
        start..end,
        &client,
        &mut record,
        &mut jitter,
    );
    assert_eq!(client.calls(), expected_calls);
    assert_eq!(
        sleeps,
        expected_sleeps
            .iter()
            .map(|value| support::decimal(value))
            .collect::<Vec<_>>()
    );
    match outcome {
        Outcome::Reloaded => assert!(support::reloaded(result).models.is_empty()),
        Outcome::Exhausted(suffix) => assert!(support::failed(result).reason.ends_with(suffix)),
        Outcome::Failed => assert_eq!(
            support::failed(result).reason,
            "model cost map fetch failed"
        ),
    }
}
