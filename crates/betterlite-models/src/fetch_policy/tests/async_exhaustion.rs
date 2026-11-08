use super::support;
use crate::fetch_policy::retry::fetch_with_retry;

#[tokio::test]
async fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![
        Ok(support::response(503, "{}")),
        Ok(support::response(503, "{}")),
        Ok(support::response(503, "{}")),
        Ok(support::response(200, "{}")),
    ]);
    let sleeper = support::RecordingSleeper::new();
    let mut jitter = support::ScriptedJitter::new(&["0.25", "0.5"]);
    let result = fetch_with_retry(&support::request(), 3, &client, &sleeper, &mut jitter).await;
    let unavailable = support::failed(result);
    assert!(unavailable.reason.starts_with("HTTP 503 from"));
    assert!(unavailable.reason.ends_with("(after 3 attempts)"));
    assert_eq!(client.calls(), 3);
    assert_eq!(
        sleeper.seconds(),
        vec![support::decimal("2.25"), support::decimal("4.5")]
    );
}
