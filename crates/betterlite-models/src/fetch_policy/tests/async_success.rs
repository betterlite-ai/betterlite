use super::support;
use crate::fetch_policy::retry::fetch_with_retry;

#[tokio::test]
async fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![Ok(support::response_with_headers(
        200,
        &[("etag", "v1")],
        r#"{"model":{}}"#,
    ))]);
    let sleeper = support::RecordingSleeper::new();
    let mut jitter = support::ScriptedJitter::new(&[]);
    let result = fetch_with_retry(&support::request(), 3, &client, &sleeper, &mut jitter).await;
    let fetched = support::reloaded(result);
    assert_eq!(fetched.etag.as_deref(), Some("v1"));
    assert_eq!(client.calls(), 1);
    assert!(sleeper.seconds().is_empty());
}
