#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_analytics::daily_metrics::{MetricWithMetadata, SpendMetrics};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::fold::{fold_entity_rollups, fold_entity_rollups_sync};
use super::support::{
    DAY_ONE, DAY_TWO, EntityParts, KEY_ONE, RepositoryCall, SENTINEL, amount, at, day,
    empty_aggregated, entity, fake, fake_failing_metadata, key_map, key_metadata_for, key_row,
    recorded_calls, window, zero_metrics,
};

fn team_metadata() -> super::EntityMetadata {
    let mut fields = IndexMap::new();
    fields.insert("team_name".to_owned(), Json::Text("Alpha".to_owned()));
    let mut metadata = super::EntityMetadata::new();
    metadata.insert("team-alpha".to_owned(), fields);
    metadata
}

#[test]
fn fold_writes_entity_metrics_onto_the_matching_date() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        rolled: true,
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert_eq!(bucket.metrics.spend, amount(5));
    assert!(bucket.metadata.is_empty());
    assert!(bucket.api_key_breakdown.is_empty());
}

#[test]
fn fold_maps_a_null_entity_id_to_unassigned() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        rolled: true,
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), None)
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("Unassigned").unwrap();
    assert_eq!(bucket.metrics.spend, amount(5));
}

#[test]
fn fold_maps_an_empty_entity_id_to_unassigned() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        rolled: true,
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some(""))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("Unassigned").unwrap();
    assert_eq!(bucket.metrics.spend, amount(5));
}

#[test]
fn fold_drops_rows_for_unknown_dates() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        rolled: true,
        spend: Some(5),
        ..EntityParts::new(Some(DAY_TWO), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    assert_eq!(results.len(), 1);
    assert!(results[0].breakdown.entities.is_empty());
}

#[test]
fn fold_drops_rows_without_a_date() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        rolled: true,
        spend: Some(5),
        ..EntityParts::new(None, Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    assert!(results[0].breakdown.entities.is_empty());
}

#[test]
fn fold_creates_a_bucket_for_sentinel_key_rows() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(SENTINEL),
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert_eq!(bucket.metrics, zero_metrics());
    assert!(bucket.api_key_breakdown.is_empty());
}

#[test]
fn fold_uses_entity_metadata_for_a_new_bucket() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        rolled: true,
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    let metadata = team_metadata();
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), Some(&metadata));
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    let mut expected = IndexMap::new();
    expected.insert("team_name".to_owned(), Json::Text("Alpha".to_owned()));
    assert_eq!(bucket.metadata, expected);
}

#[test]
fn fold_keeps_the_metadata_of_an_existing_bucket() {
    let mut results = vec![day(DAY_ONE)];
    let mut existing = IndexMap::new();
    existing.insert("kept".to_owned(), Json::Bool(true));
    results[0].breakdown.entities.insert(
        "team-alpha".to_owned(),
        MetricWithMetadata {
            metrics: SpendMetrics::default(),
            metadata: existing.clone(),
            api_key_breakdown: IndexMap::new(),
        },
    );
    let rows = [entity(EntityParts {
        rolled: true,
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    let metadata = team_metadata();
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), Some(&metadata));
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert_eq!(bucket.metadata, existing);
}

#[test]
fn fold_assigns_each_date_bucket_separately() {
    let mut results = vec![day(DAY_ONE), day(DAY_TWO)];
    let rows = [
        entity(EntityParts {
            rolled: true,
            spend: Some(1),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
        entity(EntityParts {
            rolled: true,
            spend: Some(2),
            ..EntityParts::new(Some(DAY_TWO), Some("team-alpha"))
        }),
    ];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    assert_eq!(
        results[0]
            .breakdown
            .entities
            .get("team-alpha")
            .unwrap()
            .metrics
            .spend,
        amount(1)
    );
    assert_eq!(
        results[1]
            .breakdown
            .entities
            .get("team-alpha")
            .unwrap()
            .metrics
            .spend,
        amount(2)
    );
}

#[tokio::test]
async fn fold_fetches_metadata_for_non_sentinel_entity_keys() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [
        entity(EntityParts {
            api_key: Some(KEY_ONE),
            spend: Some(5),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
        entity(EntityParts {
            api_key: Some(SENTINEL),
            spend: Some(5),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
    ];
    let repository = fake(empty_aggregated());
    fold_entity_rollups(&repository, &mut results, &rows, None)
        .await
        .unwrap();
    let calls = recorded_calls(&repository);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        RepositoryCall::Metadata {
            api_keys: BTreeSet::from([KEY_ONE.to_owned()]),
            window: Some(window(at(2024, 3, 4), at(2024, 3, 7))),
        }
    );
}

#[tokio::test]
async fn fold_skips_metadata_when_only_sentinel_entity_keys() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(SENTINEL),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    let repository = fake(empty_aggregated());
    fold_entity_rollups(&repository, &mut results, &rows, None)
        .await
        .unwrap();
    assert!(recorded_calls(&repository).is_empty());
}

#[tokio::test]
async fn fold_skips_metadata_without_entity_rows() {
    let mut results = vec![day(DAY_ONE)];
    let repository = fake(empty_aggregated());
    fold_entity_rollups(&repository, &mut results, &[], None)
        .await
        .unwrap();
    assert!(recorded_calls(&repository).is_empty());
}

#[tokio::test]
async fn fold_propagates_metadata_errors() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(KEY_ONE),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    let repository = fake_failing_metadata(empty_aggregated());
    let error = fold_entity_rollups(&repository, &mut results, &rows, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        super::DailyActivityAggregateError::Repository(_)
    ));
}

#[tokio::test]
async fn fold_attaches_key_metadata_from_the_repository() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    let mut repository = fake(empty_aggregated());
    repository.metadata = key_map(&[key_row(KEY_ONE, true)]);
    fold_entity_rollups(&repository, &mut results, &rows, None)
        .await
        .unwrap();
    let key = results[0]
        .breakdown
        .entities
        .get("team-alpha")
        .unwrap()
        .api_key_breakdown
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(key.metadata, key_metadata_for(KEY_ONE, true));
    assert_eq!(key.metrics.spend, amount(5));
}
