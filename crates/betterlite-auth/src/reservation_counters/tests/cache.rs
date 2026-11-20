use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use serde::Serialize;

use super::support::{dec, decoded_row, membership_row, org_row, project_row};
use crate::auth_cache::{AuthCache, WriteOptions, gateway_cache};
use crate::reservation_counters::cache::ReservationCounterCache as _;

type TestCache = AuthCache<DualCache<LocalMemoryOps>>;

const TTL: f64 = 60.0;

fn real_cache() -> TestCache {
    let memory = || {
        Arc::new(
            LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds"),
        )
    };
    let settings = DualCacheSettings {
        default_in_memory_ttl: Some(TTL),
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    gateway_cache(memory(), memory(), None, settings, TTL)
}

fn store<Value>(cache: &TestCache, key: &str, value: &Value)
where
    Value: Serialize,
{
    cache
        .set(Some(key), value, WriteOptions::default())
        .expect("the row write");
}

#[tokio::test]
async fn auth_cache_reads_the_project_row() {
    let cache = real_cache();
    store(
        &cache,
        "project_id:project-a",
        &project_row(r#"{"max_budget":10}"#, "1"),
    );
    let found = cache
        .read_project("project_id:project-a")
        .await
        .expect("the project read")
        .expect("the stored project");
    assert_eq!(found.project.project_id, "project-a");
    assert_eq!(found.project.spend, dec("1"));
    assert!(
        cache
            .read_project("project_id:absent")
            .await
            .expect("the absent project read")
            .is_none()
    );
}

#[tokio::test]
async fn auth_cache_reads_the_organization_row() {
    let cache = real_cache();
    store(
        &cache,
        "org_id:org-a:with_budget",
        &org_row(r#"{"max_budget":10}"#, "2"),
    );
    let found = cache
        .read_organization("org_id:org-a:with_budget")
        .await
        .expect("the organization read")
        .expect("the stored organization");
    assert_eq!(found.organization_id.as_deref(), Some("org-a"));
    assert_eq!(found.spend, dec("2"));
    assert!(
        cache
            .read_organization("org_id:absent:with_budget")
            .await
            .expect("the absent organization read")
            .is_none()
    );
}

#[tokio::test]
async fn auth_cache_reads_the_membership_row() {
    let cache = real_cache();
    store(
        &cache,
        "team_membership:user-a:team-a",
        &membership_row(r#"{"max_budget":10}"#, "3"),
    );
    let found = cache
        .read_membership("team_membership:user-a:team-a")
        .await
        .expect("the membership read")
        .expect("the stored membership");
    assert_eq!(found.user_id, "user-a");
    assert_eq!(found.team_id, "team-a");
    assert_eq!(found.spend.0, Some(dec("3")));
    assert!(
        cache
            .read_membership("team_membership:absent:absent")
            .await
            .expect("the absent membership read")
            .is_none()
    );
}

#[tokio::test]
async fn auth_cache_reads_the_default_member_budget_row() {
    let cache = real_cache();
    let row: LiteLLMBudgetTable = decoded_row(r#"{"max_budget":7}"#);
    store(&cache, "team_member_default_budget:default-1", &row);
    let found = cache
        .read_default_budget("team_member_default_budget:default-1")
        .await
        .expect("the default budget read")
        .expect("the stored default budget");
    assert_eq!(found.max_budget, Some(dec("7")));
    assert!(
        cache
            .read_default_budget("team_member_default_budget:absent")
            .await
            .expect("the absent default budget read")
            .is_none()
    );
}
