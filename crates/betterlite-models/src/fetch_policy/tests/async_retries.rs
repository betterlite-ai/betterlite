use super::support;
use crate::fetch_policy::retry::fetch_with_retry;

#[tokio::test]
async fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![
        Ok(support::response(503, "{}")),
        Ok(support::response_with_headers(
            429,
            &[("retry-after", "45")],
            "{}",
        )),
        Ok(support::response(200, "{}")),
    ]);
    let sleeper = support::RecordingSleeper::new();
    let mut jitter = support::ScriptedJitter::new(&["0.25"]);
    let result = fetch_with_retry(&support::request(), 3, &client, &sleeper, &mut jitter).await;
    assert!(support::reloaded(result).models.is_empty());
    assert_eq!(client.calls(), 3);
    assert_eq!(
        sleeper.seconds(),
        vec![support::decimal("2.25"), support::decimal("30")]
    );
}
