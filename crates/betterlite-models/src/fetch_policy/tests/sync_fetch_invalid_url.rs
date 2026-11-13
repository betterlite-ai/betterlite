use rust_decimal::Decimal;

use super::support;
use crate::fetch_policy::retry::fetch_with_retry_sync;

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![Err(support::ScriptedError::InvalidUrl {
        detail: "bad scheme".to_owned(),
    })]);
    let mut sleeps = Vec::new();
    let mut record = |seconds: Decimal| sleeps.push(seconds);
    let mut jitter = support::ScriptedJitter::new(&[]);
    let result =
        fetch_with_retry_sync(&support::request(), 1..4, &client, &mut record, &mut jitter);
    let unavailable = support::failed(result);
    assert_eq!(
        unavailable.reason,
        format!("bad scheme fetching {}", support::URL)
    );
    assert_eq!(client.calls(), 1);
    assert!(sleeps.is_empty());
}
