use std::sync::Arc;

use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::failure_hook::ports::{CallInfo, EntityType};

use super::super::{UpdateInput, update_spend_cache};
use super::support::{
    FakeCache, FakeState, HASHED_TEST_TOKEN, RecordingAlerts, key_object, update_deps,
};

async fn run(input: UpdateInput, state: FakeState) -> (Arc<FakeCache>, Arc<RecordingAlerts>) {
    let cache = FakeCache::new(state);
    let alerts = Arc::new(RecordingAlerts::default());
    let tasks = TaskTracker::new();
    let deps = update_deps(Arc::clone(&cache), Arc::clone(&alerts), &tasks);
    update_spend_cache(&input, &deps)
        .await
        .expect("the update runs");
    tasks.close();
    tasks.wait().await;
    (cache, alerts)
}

fn token_input(token: &str) -> UpdateInput {
    UpdateInput {
        token: Some(token.to_owned()),
        user_id: None,
        end_user_id: None,
        team_id: None,
        response_cost: Some(Decimal::from(100_i32)),
        span: None,
        tags: None,
    }
}

#[tokio::test]
async fn a_plaintext_key_is_hashed_before_the_lookup() {
    let (cache, _alerts) = run(token_input("sk-test-token"), FakeState::new()).await;
    assert_eq!(cache.state().auth_requests, [HASHED_TEST_TOKEN]);
}

#[tokio::test]
async fn a_stored_identifier_is_looked_up_as_given() {
    let (cache, _alerts) = run(token_input("stored-token"), FakeState::new()).await;
    assert_eq!(cache.state().auth_requests, ["stored-token"]);
}

#[tokio::test]
async fn the_soft_budget_projection_sends_one_alert_and_writes_no_key() {
    let state = FakeState::new().with_key_object(key_object(1300, Some(2000), false));
    let (cache, alerts) = run(token_input("sk-test-token"), state).await;
    let sent = alerts.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0],
        CallInfo {
            spend: Decimal::from(1400_i32),
            max_budget: Some(Decimal::from(2000_i32)),
            soft_budget: None,
            token: Some("cached-token".to_owned()),
            customer_id: None,
            user_id: Some("user-1".to_owned()),
            team_id: None,
            team_alias: None,
            organization_id: None,
            user_email: None,
            key_alias: Some("alias".to_owned()),
            projected_exceeded_date: Some("2024-06-21".to_owned()),
            projected_spend: Some(Decimal::from(2900_i32)),
            event_group: EntityType::Key,
            alert_emails: None,
            max_budget_alert_emails: None,
        }
    );
    assert!(cache.state().writes.is_empty());
}

#[tokio::test]
async fn the_soft_budget_cooldown_suppresses_the_alert() {
    let state = FakeState::new().with_key_object(key_object(1300, Some(2000), true));
    let (_cache, alerts) = run(token_input("sk-test-token"), state).await;
    assert!(alerts.sent().is_empty());
}

#[tokio::test]
async fn a_spend_inside_the_projection_sends_no_alert() {
    let state = FakeState::new().with_key_object(key_object(10, Some(100_000), false));
    let (_cache, alerts) = run(token_input("sk-test-token"), state).await;
    assert!(alerts.sent().is_empty());
}

#[tokio::test]
async fn no_soft_budget_sends_no_alert() {
    let state = FakeState::new().with_key_object(key_object(1300, None, false));
    let (_cache, alerts) = run(token_input("sk-test-token"), state).await;
    assert!(alerts.sent().is_empty());
}

#[tokio::test]
async fn a_missing_cached_key_sends_no_alert() {
    let (cache, alerts) = run(token_input("sk-test-token"), FakeState::new()).await;
    assert!(alerts.sent().is_empty());
    assert_eq!(cache.state().auth_requests.len(), 1);
}

#[tokio::test]
async fn every_request_over_the_soft_limit_alerts_again() {
    let cache =
        FakeCache::new(FakeState::new().with_key_object(key_object(1300, Some(2000), false)));
    let alerts = Arc::new(RecordingAlerts::default());
    let tasks = TaskTracker::new();
    let deps = update_deps(Arc::clone(&cache), Arc::clone(&alerts), &tasks);
    let input = token_input("sk-test-token");
    update_spend_cache(&input, &deps)
        .await
        .expect("the first update runs");
    update_spend_cache(&input, &deps)
        .await
        .expect("the second update runs");
    tasks.close();
    tasks.wait().await;
    assert_eq!(alerts.sent().len(), 2);
}
