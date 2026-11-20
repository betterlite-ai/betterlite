#![cfg(test)]

use std::collections::BTreeMap;
use std::slice::from_ref;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::keys::CacheKey;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_auth::auth_tag_budgets::rows::{BudgetRows, TagBudgetRowsFailure};
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_models::decisions::Json;

use crate::limiter_records::{TagRateLimit, TagRateLimitResolver as _};

use super::{DbTagRateLimitResolver, resolve_tag_rate_limits};

struct FakeCache {
    tags: BTreeMap<String, TagTable>,
    registry: Vec<String>,
    fail_tag_read: bool,
    reads: Mutex<Vec<String>>,
}

impl FakeCache {
    fn cached(tags: &[TagTable]) -> Self {
        Self {
            tags: tags
                .iter()
                .map(|tag| (cache_key(&tag.tag_name), tag.clone()))
                .collect(),
            registry: Vec::new(),
            fail_tag_read: false,
            reads: Mutex::new(Vec::new()),
        }
    }

    fn empty_with_registry(names: &[&str]) -> Self {
        Self {
            tags: BTreeMap::new(),
            registry: names.iter().map(|name| (*name).to_owned()).collect(),
            fail_tag_read: false,
            reads: Mutex::new(Vec::new()),
        }
    }

    fn failing_tag_read(mut self) -> Self {
        self.fail_tag_read = true;
        self
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("the cache reads lock").clone()
    }
}

#[async_trait]
impl TagBudgetCache for FakeCache {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        self.reads
            .lock()
            .expect("the cache reads lock")
            .push(key.to_owned());
        if self.fail_tag_read {
            return Err(payload_error());
        }
        Ok(self.tags.get(key).cloned())
    }

    async fn read_group(&self, _key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        Ok(None)
    }

    async fn read_registry(&self, _key: &str) -> Result<Option<Json>, AuthCacheError> {
        let names = self.registry.iter().cloned().map(Json::Text).collect();
        Ok(Some(Json::List(names)))
    }

    async fn write(
        &self,
        _key: &str,
        _value: CachedValue<'_>,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

struct FakeRows {
    tags: Vec<TagTable>,
}

#[async_trait]
impl BudgetRows<TagTable> for FakeRows {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        let found = self
            .tags
            .iter()
            .filter(|tag| names.contains(&tag.tag_name))
            .cloned()
            .collect();
        Ok(found)
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }
}

fn cache_key(name: &str) -> String {
    CacheKey::Tag(name).render()
}

fn payload_error() -> PayloadError {
    PayloadError::Json(serde_json::from_str::<i64>("not a number").expect_err("the invalid json"))
}

#[derive(serde::Serialize)]
struct BudgetWire {
    rpm_limit: Option<i64>,
    tpm_limit: Option<i64>,
}

fn tag(name: &str, budget: Option<(Option<i64>, Option<i64>)>) -> TagTable {
    let litellm_budget_table = match budget {
        None => String::new(),
        Some((rpm_limit, tpm_limit)) => {
            let table = serde_json::to_string(&BudgetWire {
                rpm_limit,
                tpm_limit,
            })
            .expect("the budget fixture serializes");
            format!(r#","litellm_budget_table":{table}"#)
        }
    };
    let wire = format!(r#"{{"tag_name":"{name}","models":[],"spend":0{litellm_budget_table}}}"#);
    serde_json::from_str(&wire).expect("the tag fixture decodes")
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn limits(map: &BTreeMap<String, TagRateLimit>) -> BTreeMap<String, (Option<i64>, Option<i64>)> {
    map.iter()
        .map(|(name, limit)| (name.clone(), (limit.rpm_limit, limit.tpm_limit)))
        .collect()
}

#[tokio::test]
async fn maps_budget_limits_for_known_tags() {
    let tags = vec![
        tag("tag-a", Some((Some(10), Some(20)))),
        tag("tag-b", Some((None, Some(5)))),
    ];
    let cache = FakeCache::empty_with_registry(&["tag-a", "tag-b"]);
    let rows = FakeRows { tags };
    let resolved = resolve_tag_rate_limits(&names(&["tag-a", "tag-b"]), true, &cache, &rows)
        .await
        .expect("the tag resolution");
    assert_eq!(
        limits(&resolved),
        BTreeMap::from([
            ("tag-a".to_owned(), (Some(10), Some(20))),
            ("tag-b".to_owned(), (None, Some(5))),
        ])
    );
}

#[tokio::test]
async fn drops_tags_without_a_budget_table() {
    let tags = vec![tag("tag-a", None)];
    let cache = FakeCache::empty_with_registry(&["tag-a"]);
    let rows = FakeRows { tags };
    let resolved = resolve_tag_rate_limits(&names(&["tag-a"]), true, &cache, &rows)
        .await
        .expect("the tag resolution");
    assert!(resolved.is_empty());
}

#[tokio::test]
async fn drops_budgets_with_no_limits() {
    let tags = vec![tag("tag-a", Some((None, None)))];
    let cache = FakeCache::empty_with_registry(&["tag-a"]);
    let rows = FakeRows { tags };
    let resolved = resolve_tag_rate_limits(&names(&["tag-a"]), true, &cache, &rows)
        .await
        .expect("the tag resolution");
    assert!(resolved.is_empty());
}

#[tokio::test]
async fn keeps_a_zero_limit() {
    let tags = vec![tag("tag-a", Some((Some(0), None)))];
    let cache = FakeCache::empty_with_registry(&["tag-a"]);
    let rows = FakeRows { tags };
    let resolved = resolve_tag_rate_limits(&names(&["tag-a"]), true, &cache, &rows)
        .await
        .expect("the tag resolution");
    assert_eq!(
        limits(&resolved),
        BTreeMap::from([("tag-a".to_owned(), (Some(0), None))])
    );
}

#[tokio::test]
async fn returns_empty_when_the_database_is_unavailable() {
    let cache = FakeCache::empty_with_registry(&["tag-a"]);
    let rows = FakeRows {
        tags: vec![tag("tag-a", Some((Some(10), None)))],
    };
    let resolved = resolve_tag_rate_limits(&names(&["tag-a"]), false, &cache, &rows)
        .await
        .expect("the tag resolution");
    assert!(resolved.is_empty());
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn returns_empty_for_no_tag_names() {
    let cache = FakeCache::empty_with_registry(&["tag-a"]);
    let rows = FakeRows { tags: Vec::new() };
    let resolved = resolve_tag_rate_limits(&[], true, &cache, &rows)
        .await
        .expect("the tag resolution");
    assert!(resolved.is_empty());
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn reads_a_cached_tag_without_the_database() {
    let cached = tag("tag-a", Some((Some(4), Some(8))));
    let cache = FakeCache::cached(from_ref(&cached));
    let rows = FakeRows { tags: Vec::new() };
    let resolved = resolve_tag_rate_limits(&names(&["tag-a"]), true, &cache, &rows)
        .await
        .expect("the tag resolution");
    assert_eq!(
        limits(&resolved),
        BTreeMap::from([("tag-a".to_owned(), (Some(4), Some(8)))])
    );
}

#[tokio::test]
async fn resolver_returns_the_resolved_limits() {
    let tags = vec![tag("tag-a", Some((Some(10), None)))];
    let cache = FakeCache::empty_with_registry(&["tag-a"]);
    let rows = FakeRows { tags };
    let resolver = DbTagRateLimitResolver {
        database: true,
        cache: &cache,
        rows: &rows,
    };
    let answer = resolver.resolve(&names(&["tag-a"])).await;
    assert_eq!(
        limits(&answer),
        BTreeMap::from([("tag-a".to_owned(), (Some(10), None))])
    );
}

#[tokio::test]
async fn resolver_fails_open_when_the_cache_read_fails() {
    let cache = FakeCache::empty_with_registry(&["tag-a"]).failing_tag_read();
    let rows = FakeRows { tags: Vec::new() };
    let resolver = DbTagRateLimitResolver {
        database: true,
        cache: &cache,
        rows: &rows,
    };
    assert!(resolver.resolve(&names(&["tag-a"])).await.is_empty());
}

#[tokio::test]
async fn resolver_fails_open_when_the_database_is_unavailable() {
    let cache = FakeCache::empty_with_registry(&["tag-a"]);
    let rows = FakeRows { tags: Vec::new() };
    let resolver = DbTagRateLimitResolver {
        database: false,
        cache: &cache,
        rows: &rows,
    };
    assert!(resolver.resolve(&names(&["tag-a"])).await.is_empty());
}
