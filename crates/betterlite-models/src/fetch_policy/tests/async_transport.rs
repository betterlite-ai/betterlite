use super::support;
use crate::fetch_policy::retry::fetch_with_retry;

#[tokio::test]
async fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![
        Err(support::ScriptedError::Transport {
            detail: "connection reset".to_owned(),
        }),
        Ok(support::response(200, "{}")),
    ]);
    let sleeper = support::RecordingSleeper::new();
    let mut jitter = support::ScriptedJitter::new(&["0.5"]);
    let result = fetch_with_retry(&support::request(), 2, &client, &sleeper, &mut jitter).await;
    assert!(support::reloaded(result).models.is_empty());
    assert_eq!(client.calls(), 2);
    assert_eq!(sleeper.seconds(), vec![support::decimal("2.5")]);
}
