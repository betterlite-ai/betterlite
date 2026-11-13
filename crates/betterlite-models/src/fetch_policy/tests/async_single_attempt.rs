use super::support;
use crate::fetch_policy::retry::fetch_with_retry;

#[tokio::test]
async fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![
        Ok(support::response(503, "{}")),
        Ok(support::response(200, "{}")),
    ]);
    let sleeper = support::RecordingSleeper::new();
    let mut jitter = support::ScriptedJitter::new(&[]);
    let result = fetch_with_retry(&support::request(), 1, &client, &sleeper, &mut jitter).await;
    let unavailable = support::failed(result);
    assert!(unavailable.reason.ends_with("(after 1 attempts)"));
    assert_eq!(client.calls(), 1);
    assert!(sleeper.seconds().is_empty());
}
