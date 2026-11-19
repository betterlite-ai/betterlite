use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::support::{
    GROUP_A, GROUP_B, GROUP_C, GROUP_ZERO_BUDGET, RecordingCache, RecordingRows, TTL, group_budget,
    group_row,
};
use crate::auth_read::include::{Including, IncludingQuery};
use crate::auth_tag_budgets::TagBudgetError;
use crate::auth_tag_budgets::groups::{
    ModelAccessGroupBudget, group_budgets_batch, model_access_group_budget,
};
use crate::auth_tag_budgets::rows::BudgetInclude;
use crate::auth_tag_budgets::rows::group::ModelAccessGroupBudgetTable;

async fn batch(
    cache: &RecordingCache,
    rows: &RecordingRows,
    names: &[&str],
) -> Result<BTreeMap<String, ModelAccessGroupBudget>, TagBudgetError> {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    group_budgets_batch(&names, true, cache, rows).await
}

#[tokio::test]
async fn absent_db_empty() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), Vec::new());
    let names = vec!["group-a".to_owned()];
    let found = group_budgets_batch(&names, false, &cache, &rows)
        .await
        .expect("the absent database fails open");
    assert!(found.is_empty());
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
    assert!(rows.group_queries().is_empty());
}

#[tokio::test]
async fn empty_batch_no_io() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]);
    let found = batch(&cache, &rows, &[])
        .await
        .expect("the empty batch returns nothing");
    assert!(found.is_empty());
    assert!(cache.reads().is_empty());
    assert!(rows.group_queries().is_empty());
}

#[tokio::test]
async fn ordered_duplicate_probes() {
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A), group_row(GROUP_B)]);
    let cache = RecordingCache::new(TTL);
    let found = batch(&cache, &rows, &["group-b", "group-a", "group-b"])
        .await
        .expect("the batch read");
    assert_eq!(found.len(), 2);
    assert_eq!(
        cache.tagged_reads("model_access_group:"),
        [
            "model_access_group:group-b",
            "model_access_group:group-a",
            "model_access_group:group-b"
        ]
    );
    assert_eq!(
        rows.group_queries(),
        vec![vec!["group-b", "group-a", "group-b"]]
    );
}

#[tokio::test]
async fn cache_hits_survive_failed_fetch() {
    let cache = RecordingCache::new(TTL).with_group(group_budget(GROUP_A));
    let rows = RecordingRows::new(Vec::new(), Vec::new()).failing_groups();
    let found = batch(&cache, &rows, &["group-a", "group-b"])
        .await
        .expect("the failed fetch keeps the cache hit");
    assert_eq!(found.keys().cloned().collect::<Vec<_>>(), ["group-a"]);
    assert_eq!(rows.group_queries(), vec![vec!["group-b"]]);
}

#[tokio::test]
async fn cache_read_error_propagates() {
    let cache = RecordingCache::new(TTL).failing_group_read();
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]);
    let error = batch(&cache, &rows, &["group-a"])
        .await
        .expect_err("the cache read failure reaches the caller");
    assert!(matches!(error, TagBudgetError::GroupPayload(_)));
    assert!(rows.group_queries().is_empty());
}

#[tokio::test]
async fn undecodable_payload_refetched() {
    let raw = Json::Text("not a budget".to_owned());
    let cache = RecordingCache::new(TTL).with_registry("model_access_group:group-a", raw);
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]);
    let found = batch(&cache, &rows, &["group-a"])
        .await
        .expect("an undecodable payload is a miss, not a failure");
    assert_eq!(found.len(), 1);
    assert_eq!(rows.group_queries(), vec![vec!["group-a"]]);
}

#[tokio::test]
async fn one_query_for_misses() {
    let rows = RecordingRows::new(
        Vec::new(),
        vec![group_row(GROUP_A), group_row(GROUP_B), group_row(GROUP_C)],
    );
    let cache = RecordingCache::new(TTL).with_group(group_budget(GROUP_A));
    let found = batch(&cache, &rows, &["group-a", "group-b", "group-c"])
        .await
        .expect("the batch read");
    assert_eq!(found.len(), 3);
    assert_eq!(rows.group_queries(), vec![vec!["group-b", "group-c"]]);
}

#[tokio::test]
async fn all_hits_skip_the_database() {
    let cache = RecordingCache::new(TTL).with_group(group_budget(GROUP_A));
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]);
    let found = batch(&cache, &rows, &["group-a"])
        .await
        .expect("the cached budget is served without a row query");
    assert_eq!(found.len(), 1);
    assert!(rows.group_queries().is_empty());
}

#[test]
fn requested_names_and_budget_include() {
    let names = vec!["group-a".to_owned(), "group-b".to_owned()];
    let limit = Some(9);
    let include = BudgetInclude {
        litellm_budget_table: true,
    };
    let with_budget = ModelAccessGroupBudgetTable.including(
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
            .contains("entry.\"access_group_name\" = ANY($1::text[])")
    );
    assert!(with_budget.sql.contains("'litellm_budget_table'"));
    let plain = ModelAccessGroupBudgetTable.including(
        IncludingQuery::Listed {
            filter: Some(&names),
            limit: &limit,
        },
        None,
    );
    assert_eq!(plain.parameters.len(), 2);
    assert!(!plain.sql.contains("litellm_budget_table"));
}

#[tokio::test]
async fn fetched_row_wins() {
    let cached = r#"{"access_group_name":"group-a","spend":1}"#;
    let fetched = r#"{"access_group_name":"group-a","spend":9}"#;
    let cache = RecordingCache::new(TTL).with_group_hit_once(group_budget(cached));
    let rows = RecordingRows::new(Vec::new(), vec![group_row(fetched)]);
    let found = batch(&cache, &rows, &["group-a", "group-a"])
        .await
        .expect("the fetched row overwrites the cached hit");
    assert_eq!(found["group-a"].spend, group_budget(fetched).spend);
}

#[tokio::test]
async fn missing_rows_omitted() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]);
    let found = batch(&cache, &rows, &["group-a", "group-b"])
        .await
        .expect("the batch read");
    assert_eq!(found.keys().cloned().collect::<Vec<_>>(), ["group-a"]);
    assert_eq!(
        cache.prefixed_writes("model_access_group:"),
        [("model_access_group:group-a".to_owned(), TTL)]
    );
}

#[tokio::test]
async fn query_failure_debug_drop() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]).failing_groups();
    let found = batch(&cache, &rows, &["group-a"])
        .await
        .expect("the query failure fails open");
    assert!(found.is_empty());
    assert_eq!(rows.group_queries(), vec![vec!["group-a"]]);
    assert!(cache.prefixed_writes("model_access_group:").is_empty());
}

#[tokio::test]
async fn decode_failure_debug_drop() {
    let cache = RecordingCache::new(TTL);
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]).undecodable_groups();
    let found = batch(&cache, &rows, &["group-a"])
        .await
        .expect("the row conversion failure fails open");
    assert!(found.is_empty());
    assert!(cache.prefixed_writes("model_access_group:").is_empty());
}

#[tokio::test]
async fn cache_failure_keeps_prior_writes() {
    let cache = RecordingCache::new(TTL).failing_writes_after(1);
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A), group_row(GROUP_B)]);
    let found = batch(&cache, &rows, &["group-a", "group-b"])
        .await
        .expect("the cache write failure fails open");
    assert!(found.is_empty());
    assert_eq!(
        cache.prefixed_writes("model_access_group:"),
        [("model_access_group:group-a".to_owned(), TTL)]
    );
}

#[tokio::test]
async fn management_ttl_used() {
    let cache = RecordingCache::new(42.0);
    let rows = RecordingRows::new(Vec::new(), vec![group_row(GROUP_A)]);
    let found = batch(&cache, &rows, &["group-a"])
        .await
        .expect("the batch read");
    assert_eq!(found.len(), 1);
    assert_eq!(
        cache.prefixed_writes("model_access_group:"),
        [("model_access_group:group-a".to_owned(), 42.0)]
    );
}

#[test]
fn null_spend_zero() {
    let row = group_row(r#"{"access_group_name":"group-a","spend":null}"#);
    let budget = model_access_group_budget(&row);
    assert_eq!(budget.spend, Decimal::ZERO);
}

#[test]
fn absent_budget_none() {
    let row = group_row(r#"{"access_group_name":"group-a","spend":2}"#);
    let budget = model_access_group_budget(&row);
    assert_eq!(budget.spend, Decimal::new(2, 0));
    assert_eq!(budget.max_budget, None);
}

#[test]
fn zero_budget_retained() {
    let budget = model_access_group_budget(&group_row(GROUP_ZERO_BUDGET));
    assert_eq!(budget.max_budget, Some(Decimal::ZERO));
}
