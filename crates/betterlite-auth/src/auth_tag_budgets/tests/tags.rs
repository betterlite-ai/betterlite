use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::tag::TagTable;

use super::support::{
    RecordingCache, RecordingRows, TAG_A, TAG_B, TAG_C, TAG_REGISTRY_KEY, TTL, registry_names, tag,
};
use crate::auth_read::include::{Including, IncludingQuery};
use crate::auth_tag_budgets::TagBudgetError;
use crate::auth_tag_budgets::rows::BudgetInclude;
use crate::auth_tag_budgets::rows::tag::TagBudgetTable;
use crate::auth_tag_budgets::tags::get_tag_objects_batch;

async fn batch(
    cache: &RecordingCache,
    rows: &RecordingRows,
    names: &[&str],
) -> Result<BTreeMap<String, TagTable>, TagBudgetError> {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    get_tag_objects_batch(&names, true, cache, rows).await
}

#[tokio::test]
async fn absent_db_empty() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), Vec::new());
    let names = vec!["tag-a".to_owned()];
    let found = get_tag_objects_batch(&names, false, &cache, &rows)
        .await
        .expect("the absent database fails open");
    assert!(found.is_empty());
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
    assert!(rows.tag_queries().is_empty());
    assert!(rows.registry_takes().is_empty());
}

#[tokio::test]
async fn empty_batch_no_io() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new());
    let found = batch(&cache, &rows, &[])
        .await
        .expect("the empty batch returns nothing");
    assert!(found.is_empty());
    assert!(cache.reads().is_empty());
    assert!(rows.tag_queries().is_empty());
    assert!(rows.registry_takes().is_empty());
}

#[tokio::test]
async fn ordered_duplicate_probes() {
    let cache = RecordingCache::new(TTL)
        .with_registry(TAG_REGISTRY_KEY, registry_names(&["tag-a", "tag-b"]));
    let rows = RecordingRows::new(vec![tag(TAG_A), tag(TAG_B)], Vec::new());
    let found = batch(&cache, &rows, &["tag-b", "tag-a", "tag-b"])
        .await
        .expect("the batch read");
    assert_eq!(found.len(), 2);
    assert_eq!(
        cache.tagged_reads("tag:"),
        ["tag:tag-b", "tag:tag-a", "tag:tag-b"]
    );
    assert_eq!(rows.tag_queries(), vec![vec!["tag-b", "tag-a", "tag-b"]]);
}

#[tokio::test]
async fn cache_hits_survive_failed_fetch() {
    let cache = RecordingCache::new(TTL).with_tag(tag(TAG_A));
    let rows = RecordingRows::new(Vec::new(), Vec::new()).failing_tags();
    let found = batch(&cache, &rows, &["tag-a", "tag-b"])
        .await
        .expect("the failed fetch keeps the cache hit");
    assert_eq!(found.keys().cloned().collect::<Vec<_>>(), ["tag-a"]);
    assert_eq!(rows.tag_queries(), vec![vec!["tag-b"]]);
}

#[tokio::test]
async fn cache_read_error_propagates() {
    let cache = RecordingCache::new(TTL).failing_tag_read();
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new());
    let error = batch(&cache, &rows, &["tag-a"])
        .await
        .expect_err("the cache read failure reaches the caller");
    assert!(matches!(error, TagBudgetError::TagPayload(_)));
    assert!(rows.tag_queries().is_empty());
    assert!(rows.registry_takes().is_empty());
}

#[tokio::test]
async fn one_query_for_misses() {
    let cache = RecordingCache::new(TTL).with_tag(tag(TAG_A)).with_registry(
        TAG_REGISTRY_KEY,
        registry_names(&["tag-a", "tag-b", "tag-c"]),
    );
    let rows = RecordingRows::new(vec![tag(TAG_A), tag(TAG_B), tag(TAG_C)], Vec::new());
    let found = batch(&cache, &rows, &["tag-a", "tag-b", "tag-c"])
        .await
        .expect("the batch read");
    assert_eq!(found.len(), 3);
    assert_eq!(rows.tag_queries(), vec![vec!["tag-b", "tag-c"]]);
}

#[tokio::test]
async fn all_hits_skip_the_registry() {
    let cache = RecordingCache::new(TTL).with_tag(tag(TAG_A));
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new());
    let found = batch(&cache, &rows, &["tag-a"])
        .await
        .expect("the cached tag is served without a registry read");
    assert_eq!(found.len(), 1);
    assert!(rows.tag_queries().is_empty());
    assert!(rows.registry_takes().is_empty());
}

#[test]
fn requested_names_and_budget_include() {
    let names = vec!["tag-a".to_owned(), "tag-b".to_owned()];
    let limit = Some(9);
    let include = BudgetInclude {
        litellm_budget_table: true,
    };
    let with_budget = TagBudgetTable.including(
        IncludingQuery::Listed {
            filter: Some(&names),
            limit: &limit,
        },
        Some(&include),
    );
    assert_eq!(with_budget.parameters.len(), 2);
    assert!(
        with_budget
            .sql
            .contains("entry.\"tag_name\" = ANY($1::text[])")
    );
    assert!(with_budget.sql.contains("'litellm_budget_table'"));
    let plain = TagBudgetTable.including(
        IncludingQuery::Listed {
            filter: Some(&names),
            limit: &limit,
        },
        None,
    );
    assert_eq!(plain.parameters.len(), 2);
    assert!(plain.sql.contains("entry.\"tag_name\" = ANY($1::text[])"));
    assert!(!plain.sql.contains("litellm_budget_table"));
}

#[tokio::test]
async fn fetched_row_wins() {
    let cached = r#"{"tag_name":"tag-a","models":[],"spend":1}"#;
    let fetched = tag(r#"{"tag_name":"tag-a","models":["m9"],"spend":9}"#);
    let cache = RecordingCache::new(TTL)
        .with_tag_hit_once(tag(cached))
        .with_registry(TAG_REGISTRY_KEY, registry_names(&["tag-a"]));
    let rows = RecordingRows::new(vec![fetched.clone()], Vec::new());
    let found = batch(&cache, &rows, &["tag-a", "tag-a"])
        .await
        .expect("the fetched row overwrites the cached hit");
    assert_eq!(found["tag-a"].spend, fetched.spend);
    assert_eq!(found["tag-a"].models, ["m9"]);
}

#[tokio::test]
async fn missing_rows_omitted() {
    let cache = RecordingCache::new(TTL)
        .with_registry(TAG_REGISTRY_KEY, registry_names(&["tag-a", "tag-b"]));
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new());
    let found = batch(&cache, &rows, &["tag-a", "tag-b"])
        .await
        .expect("the batch read");
    assert_eq!(found.keys().cloned().collect::<Vec<_>>(), ["tag-a"]);
    assert_eq!(
        cache.prefixed_writes("tag:"),
        [("tag:tag-a".to_owned(), TTL)]
    );
}

#[tokio::test]
async fn query_failure_debug_drop() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new()).failing_tags();
    let found = batch(&cache, &rows, &["tag-a"])
        .await
        .expect("the query failure fails open");
    assert!(found.is_empty());
    assert_eq!(rows.tag_queries(), vec![vec!["tag-a"]]);
    assert!(cache.prefixed_writes("tag:").is_empty());
}

#[tokio::test]
async fn decode_failure_debug_drop() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new()).undecodable_tags();
    let found = batch(&cache, &rows, &["tag-a"])
        .await
        .expect("the row conversion failure fails open");
    assert!(found.is_empty());
    assert!(cache.prefixed_writes("tag:").is_empty());
}

#[tokio::test]
async fn cache_failure_keeps_prior_writes() {
    let cache = RecordingCache::new(TTL)
        .with_registry(TAG_REGISTRY_KEY, registry_names(&["tag-a", "tag-b"]))
        .failing_writes_after(1);
    let rows = RecordingRows::new(vec![tag(TAG_A), tag(TAG_B)], Vec::new());
    let found = batch(&cache, &rows, &["tag-a", "tag-b"])
        .await
        .expect("the cache write failure fails open");
    assert!(found.is_empty());
    assert_eq!(
        cache.prefixed_writes("tag:"),
        [("tag:tag-a".to_owned(), TTL)]
    );
}

#[tokio::test]
async fn management_ttl_used() {
    let cache =
        RecordingCache::new(42.0).with_registry(TAG_REGISTRY_KEY, registry_names(&["tag-a"]));
    let rows = RecordingRows::new(vec![tag(TAG_A)], Vec::new());
    let found = batch(&cache, &rows, &["tag-a"])
        .await
        .expect("the batch read");
    assert_eq!(found.len(), 1);
    assert_eq!(
        cache.prefixed_writes("tag:"),
        [("tag:tag-a".to_owned(), 42.0)]
    );
}
