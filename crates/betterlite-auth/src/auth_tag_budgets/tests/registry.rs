use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_models::decisions::Json;
use tokio::sync::Notify;

use super::support::{
    GROUP_A, GROUP_REGISTRY_KEY, RecordingCache, RecordingRows, TAG_A, TAG_B, TAG_REGISTRY_KEY,
    TTL, group_row, named_group_rows, named_tags, registry_names, tag,
};
use crate::auth_tag_budgets::TagBudgetError;
use crate::auth_tag_budgets::groups::get_model_access_group_budgets_batch;
use crate::auth_tag_budgets::registry::{
    GROUP_REGISTRY_TAKE, TAG_REGISTRY_TAKE, load_model_access_group_registry, load_tag_registry,
};
use crate::auth_tag_budgets::rows::{
    GroupRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure, TagRows,
};
use crate::auth_tag_budgets::tags::get_tag_objects_batch;

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[tokio::test]
async fn tag_registry_take_5001() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(named_tags(2), Vec::new());
    let known = load_tag_registry(&cache, &rows)
        .await
        .expect("the tag registry loads");
    assert_eq!(rows.registry_takes(), [TAG_REGISTRY_TAKE]);
    assert_eq!(known, Some(BTreeSet::from(names(&["tag-0", "tag-1"]))));
    let answer = Json::List(vec![
        Json::Text("tag-0".to_owned()),
        Json::Text("tag-1".to_owned()),
    ]);
    assert_eq!(cache.registry_answer(TAG_REGISTRY_KEY), Some(answer));
    assert_eq!(
        cache.prefixed_writes(TAG_REGISTRY_KEY),
        [(TAG_REGISTRY_KEY.to_owned(), TTL)]
    );
}

#[tokio::test]
async fn group_registry_take_5001() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), named_group_rows(1));
    let known = load_model_access_group_registry(&cache, &rows)
        .await
        .expect("the group registry loads");
    assert_eq!(rows.group_registry_takes(), [GROUP_REGISTRY_TAKE]);
    assert_eq!(known, Some(BTreeSet::from(names(&["group-0"]))));
    assert_eq!(
        cache.prefixed_writes(GROUP_REGISTRY_KEY),
        [(GROUP_REGISTRY_KEY.to_owned(), TTL)]
    );
}

struct GatedRows {
    tag_started: Arc<Notify>,
    group_started: Arc<Notify>,
}

#[async_trait]
impl TagRows for GatedRows {
    async fn by_names(&self, _names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        self.tag_started.notify_one();
        self.group_started.notified().await;
        Ok(vec![tag(TAG_A)])
    }
}

#[async_trait]
impl GroupRows for GatedRows {
    async fn by_names(
        &self,
        _names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }

    async fn registry_rows(
        &self,
        _take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        self.tag_started.notified().await;
        self.group_started.notify_one();
        Ok(vec![group_row(GROUP_A)])
    }
}

#[tokio::test]
async fn distinct_registry_keys_locks() {
    let cache = RecordingCache::new(TTL);
    let rows = GatedRows {
        tag_started: Arc::new(Notify::new()),
        group_started: Arc::new(Notify::new()),
    };
    let loads = async {
        tokio::join!(
            load_tag_registry(&cache, &rows),
            load_model_access_group_registry(&cache, &rows)
        )
    };
    let (tags, groups) = tokio::time::timeout(Duration::from_secs(2), loads)
        .await
        .expect("each registry takes its own load lock");
    assert_eq!(
        tags.expect("the tag registry loads"),
        Some(BTreeSet::from(names(&["tag-a"])))
    );
    assert_eq!(
        groups.expect("the group registry loads"),
        Some(BTreeSet::from(names(&["group-a"])))
    );
    assert!(cache.registry_answer(TAG_REGISTRY_KEY).is_some());
    assert!(cache.registry_answer(GROUP_REGISTRY_KEY).is_some());
}

#[tokio::test]
async fn known_registry_filters_tags() {
    let cache = RecordingCache::new(TTL)
        .with_registry(TAG_REGISTRY_KEY, registry_names(&["tag-a", "tag-c"]));
    let rows = RecordingRows::new(vec![tag(TAG_A), tag(TAG_B)], Vec::new());
    let found = get_tag_objects_batch(&names(&["tag-a", "tag-b"]), true, &cache, &rows)
        .await
        .expect("the batch read");
    assert_eq!(found.keys().cloned().collect::<Vec<_>>(), ["tag-a"]);
    assert_eq!(rows.tag_queries(), vec![names(&["tag-a"])]);
    assert!(rows.registry_takes().is_empty());
}

#[tokio::test]
async fn empty_registry_skips_query() {
    let cache = RecordingCache::new(TTL).with_registry(TAG_REGISTRY_KEY, registry_names(&[]));
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new());
    let found = get_tag_objects_batch(&names(&["tag-a", "tag-b"]), true, &cache, &rows)
        .await
        .expect("the batch read");
    assert!(found.is_empty());
    assert!(rows.tag_queries().is_empty());
    assert!(cache.prefixed_writes("tag:").is_empty());
}

#[tokio::test]
async fn unavailable_registry_fetches_all() {
    let sentinel = Json::Text("__tag_registry_overflow__".to_owned());
    let cache = RecordingCache::new(TTL).with_registry(TAG_REGISTRY_KEY, sentinel);
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new());
    let found = get_tag_objects_batch(&names(&["tag-a", "tag-b"]), true, &cache, &rows)
        .await
        .expect("the batch read");
    assert_eq!(found.keys().cloned().collect::<Vec<_>>(), ["tag-a"]);
    assert_eq!(rows.tag_queries(), vec![names(&["tag-a", "tag-b"])]);
    assert!(rows.registry_takes().is_empty());
}

#[tokio::test]
async fn overflow_fetches_all() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(named_tags(5001), Vec::new());
    let found = get_tag_objects_batch(&names(&["tag-0", "tag-outside"]), true, &cache, &rows)
        .await
        .expect("the batch read");
    assert_eq!(found.keys().cloned().collect::<Vec<_>>(), ["tag-0"]);
    assert_eq!(rows.registry_takes(), [TAG_REGISTRY_TAKE]);
    assert_eq!(rows.tag_queries(), vec![names(&["tag-0", "tag-outside"])]);
    let sentinel = Json::Text("__tag_registry_overflow__".to_owned());
    assert_eq!(cache.registry_answer(TAG_REGISTRY_KEY), Some(sentinel));
}

#[tokio::test]
async fn group_overflow_sentinel() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), named_group_rows(5001));
    let known = load_model_access_group_registry(&cache, &rows)
        .await
        .expect("the group registry loader");
    assert_eq!(known, None);
    let sentinel = Json::Text("__model_access_group_registry_overflow__".to_owned());
    assert_eq!(cache.registry_answer(GROUP_REGISTRY_KEY), Some(sentinel));
}

#[tokio::test]
async fn registry_error_propagates() {
    let cache = RecordingCache::new(TTL).failing_registry_read();
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new());
    let error = get_tag_objects_batch(&names(&["tag-a"]), true, &cache, &rows)
        .await
        .expect_err("the registry failure reaches the caller");
    assert!(matches!(error, TagBudgetError::Registry(_)));
    assert!(rows.tag_queries().is_empty());
    assert!(rows.registry_takes().is_empty());
}

#[tokio::test]
async fn group_registry_error_propagates() {
    let cache = RecordingCache::new(TTL).failing_registry_read();
    let rows = RecordingRows::new(Vec::new(), named_group_rows(1));
    let error = load_model_access_group_registry(&cache, &rows)
        .await
        .expect_err("the group registry failure reaches the caller");
    assert_eq!(
        error.to_string(),
        "the registry could not be read from the cache"
    );
    assert!(rows.group_registry_takes().is_empty());
}

#[tokio::test]
async fn group_fetch_does_not_load_registry() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]);
    let found = get_model_access_group_budgets_batch(&names(&["group-a"]), true, &cache, &rows)
        .await
        .expect("the batch read");
    assert_eq!(found.len(), 1);
    assert!(rows.group_registry_takes().is_empty());
    assert!(cache.tagged_reads(GROUP_REGISTRY_KEY).is_empty());
    assert_eq!(rows.group_queries(), vec![names(&["group-a"])]);
}
