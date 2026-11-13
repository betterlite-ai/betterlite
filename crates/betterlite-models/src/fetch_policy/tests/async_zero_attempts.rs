use super::support;
use crate::fetch_policy::retry::fetch_with_retry;

#[tokio::test]
async fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![Ok(support::response(200, "{}"))]);
    let sleeper = support::RecordingSleeper::new();
    let mut jitter = support::ScriptedJitter::new(&[]);
    let result = fetch_with_retry(&support::request(), 0, &client, &sleeper, &mut jitter).await;
    assert_eq!(
        support::failed(result).reason,
        "model cost map fetch failed"
    );
    assert_eq!(client.calls(), 0);
    assert!(sleeper.seconds().is_empty());
}
