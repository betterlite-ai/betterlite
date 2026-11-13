use super::support;
use crate::fetch_policy::retry::fetch_with_retry;

#[tokio::test]
async fn catalog_fetch_policy_behavior_tests_behavior() {
    let client = support::ScriptedClient::new(vec![Err(support::ScriptedError::InvalidUrl {
        detail: "bad scheme".to_owned(),
    })]);
    let sleeper = support::RecordingSleeper::new();
    let mut jitter = support::ScriptedJitter::new(&[]);
    let result = fetch_with_retry(&support::request(), 3, &client, &sleeper, &mut jitter).await;
    assert_eq!(
        support::failed(result).reason,
        format!("bad scheme fetching {}", support::URL)
    );
    assert_eq!(client.calls(), 1);
    assert!(sleeper.seconds().is_empty());
}
