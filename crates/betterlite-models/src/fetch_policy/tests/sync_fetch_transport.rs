use rust_decimal::Decimal;

use super::support;
use crate::fetch_policy::retry::fetch_with_retry_sync;

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![
        Err(support::ScriptedError::Transport {
            detail: "connection reset".to_owned(),
        }),
        Ok(support::response(200, "{}")),
    ]);
    let mut sleeps = Vec::new();
    let mut record = |seconds: Decimal| sleeps.push(seconds);
    let mut jitter = support::ScriptedJitter::new(&["0.25"]);
    let result =
        fetch_with_retry_sync(&support::request(), 1..3, &client, &mut record, &mut jitter);
    assert!(support::reloaded(result).models.is_empty());
    assert_eq!(client.calls(), 2);
    assert_eq!(sleeps, vec![support::decimal("2.25")]);
}
