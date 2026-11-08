use super::support;
use crate::fetch_policy::retry::fetch_with_retry;

#[tokio::test]
async fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![
        Ok(support::response(404, "{}")),
        Ok(support::response(200, "{}")),
    ]);
    let sleeper = support::RecordingSleeper::new();
    let mut jitter = support::ScriptedJitter::new(&[]);
    let result = fetch_with_retry(&support::request(), 3, &client, &sleeper, &mut jitter).await;
    assert_eq!(
        support::failed(result).reason,
        format!("HTTP 404 from {}", support::URL)
    );
    assert_eq!(client.calls(), 1);
    assert!(sleeper.seconds().is_empty());
}
