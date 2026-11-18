use std::sync::Arc;

use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::WriteOptions;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::DualCache;
use betterlite_db::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use super::super::cache::SpendCache;
use super::super::{UpdateInput, update_spend_cache};
use super::support::{HASHED_TEST_TOKEN, RecordingAlerts, json_from, local_cache, update_deps};

fn fixtures() -> [(&'static str, Json); 5] {
    [
        ("user-1", json_from(r#"{"user_id":"user-1","spend":1}"#)),
        (GLOBAL_PROXY_SPEND_CACHE_KEY, json_from("10")),
        (
            "end_user_id:end-1",
            json_from(r#"{"user_id":"end-1","blocked":false,"spend":2}"#),
        ),
        (
            "team_id:team-1",
            json_from(r#"{"team_id":"team-1","spend":3,"last_refreshed_at":null}"#),
        ),
        (
            "tag:qa",
            json_from(r#"{"tag_name":"qa","models":[],"spend":4}"#),
        ),
    ]
}

async fn seed(cache: &AuthCache<DualCache<LocalMemoryOps>>) {
    for (key, value) in fixtures() {
        cache
            .async_set(Some(key), &value, WriteOptions::default())
            .await
            .expect("the fixture is seeded");
    }
}

#[tokio::test]
async fn the_real_cache_stores_the_updated_objects_and_the_global_spend() {
    let cache = Arc::new(local_cache());
    seed(&cache).await;
    let alerts = Arc::new(RecordingAlerts::default());
    let tasks = TaskTracker::new();
    let deps = update_deps(Arc::clone(&cache), Arc::clone(&alerts), &tasks);
    let input = UpdateInput {
        token: None,
        user_id: Some("user-1".to_owned()),
        end_user_id: Some("end-1".to_owned()),
        team_id: Some("team-1".to_owned()),
        response_cost: Some(Decimal::from(5)),
        span: None,
        tags: Some(vec![Json::Text("qa".to_owned())]),
    };
    update_spend_cache(&input, &deps)
        .await
        .expect("the update runs");
    tasks.close();
    tasks.wait().await;
    let user = cache
        .async_get_typed::<LiteLLMUserTable>("user-1", true)
        .await
        .expect("the user reads")
        .expect("the user is cached");
    assert_eq!(user.spend, Decimal::from(6));
    let end_user = cache
        .async_get_typed::<LiteLLMEndUserTable>("end_user_id:end-1", true)
        .await
        .expect("the end user reads")
        .expect("the end user is cached");
    assert_eq!(end_user.spend, Decimal::from(7));
    let team = cache
        .async_get_typed::<LiteLLMTeamTableCachedObj>("team_id:team-1", true)
        .await
        .expect("the team reads")
        .expect("the team is cached");
    assert_eq!(team.team.fields.spend, Some(Decimal::from(8)));
    let tag = cache
        .async_get_typed::<TagTable>("tag:qa", true)
        .await
        .expect("the tag reads")
        .expect("the tag is cached");
    assert_eq!(tag.spend, Decimal::from(9));
    let global = cache
        .async_get(GLOBAL_PROXY_SPEND_CACHE_KEY, true)
        .await
        .expect("the global spend is cached");
    assert_eq!(global, json_from("15"));
}

#[tokio::test]
async fn a_cached_key_with_a_null_spend_is_a_miss_and_sends_no_alert() {
    let cache = Arc::new(local_cache());
    cache
        .async_set(
            Some(HASHED_TEST_TOKEN),
            &json_from(
                r#"{"token":"cached-token","user_id":"user-1","spend":null,"soft_budget":50,"soft_budget_cooldown":false}"#,
            ),
            WriteOptions::default(),
        )
        .await
        .expect("the key fixture is seeded");
    let alerts = Arc::new(RecordingAlerts::default());
    let tasks = TaskTracker::new();
    let deps = update_deps(Arc::clone(&cache), Arc::clone(&alerts), &tasks);
    let input = UpdateInput {
        token: Some("sk-test-token".to_owned()),
        user_id: None,
        end_user_id: None,
        team_id: None,
        response_cost: Some(Decimal::from(100)),
        span: None,
        tags: None,
    };
    update_spend_cache(&input, &deps)
        .await
        .expect("the update runs");
    tasks.close();
    tasks.wait().await;
    assert!(alerts.sent().is_empty());
}

#[test]
fn the_real_cache_reports_the_management_object_ttl() {
    let cache = local_cache();
    assert_eq!(cache.management_ttl().to_bits(), 45.0_f64.to_bits());
}

#[tokio::test]
async fn the_real_cache_reads_the_cached_key_and_sends_the_projected_alert() {
    let cache = Arc::new(local_cache());
    cache
        .async_set(
            Some(HASHED_TEST_TOKEN),
            &json_from(
                r#"{"token":"cached-token","user_id":"user-1","spend":1300,"soft_budget":2000,"soft_budget_cooldown":false}"#,
            ),
            WriteOptions::default(),
        )
        .await
        .expect("the key fixture is seeded");
    let alerts = Arc::new(RecordingAlerts::default());
    let tasks = TaskTracker::new();
    let deps = update_deps(Arc::clone(&cache), Arc::clone(&alerts), &tasks);
    let input = UpdateInput {
        token: Some("sk-test-token".to_owned()),
        user_id: None,
        end_user_id: None,
        team_id: None,
        response_cost: Some(Decimal::from(100)),
        span: None,
        tags: None,
    };
    update_spend_cache(&input, &deps)
        .await
        .expect("the update runs");
    tasks.close();
    tasks.wait().await;
    let sent = alerts.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].spend, Decimal::from(1400));
    assert_eq!(sent[0].max_budget, Some(Decimal::from(2000)));
}
