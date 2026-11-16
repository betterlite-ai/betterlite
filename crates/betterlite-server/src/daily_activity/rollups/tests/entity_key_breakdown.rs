#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::fold::fold_entity_rollups_sync;
use super::support::{
    DAY_ONE, DAY_TWO, EntityParts, KEY_ONE, SENTINEL, amount, day, entity, key_map,
    key_metadata_for, key_row, missing_key_metadata, zero_metrics,
};

#[test]
fn rolled_row_assigns_the_entity_total_metrics() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        rolled: true,
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert_eq!(bucket.metrics.spend, amount(5));
}

#[test]
fn rolled_row_skips_the_key_breakdown() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(KEY_ONE),
        rolled: true,
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert!(bucket.api_key_breakdown.is_empty());
}

#[test]
fn rolled_row_with_a_sentinel_key_assigns_total_metrics() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(SENTINEL),
        rolled: true,
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert_eq!(bucket.metrics.spend, amount(5));
}

#[test]
fn key_row_assigns_a_key_breakdown() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert_eq!(bucket.metrics, zero_metrics());
    assert_eq!(
        bucket.api_key_breakdown.get(KEY_ONE).unwrap().metrics.spend,
        amount(5)
    );
}

#[test]
fn key_row_skips_the_ptu_sentinel() {
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
fn key_row_without_an_api_key_leaves_the_bucket_empty() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        spend: Some(5),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert_eq!(bucket.metrics, zero_metrics());
    assert!(bucket.api_key_breakdown.is_empty());
}

#[test]
fn key_row_attaches_key_metadata() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(KEY_ONE),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    let metadata = key_map(&[key_row(KEY_ONE, true)]);
    fold_entity_rollups_sync(&mut results, &rows, &metadata, None);
    let key = results[0]
        .breakdown
        .entities
        .get("team-alpha")
        .unwrap()
        .api_key_breakdown
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(key.metadata, key_metadata_for(KEY_ONE, true));
}

#[test]
fn key_row_marks_a_missing_key() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [entity(EntityParts {
        api_key: Some(KEY_ONE),
        ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
    })];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let key = results[0]
        .breakdown
        .entities
        .get("team-alpha")
        .unwrap()
        .api_key_breakdown
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(key.metadata, missing_key_metadata());
}

#[test]
fn key_row_replaces_a_previous_key_row() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [
        entity(EntityParts {
            api_key: Some(KEY_ONE),
            spend: Some(1),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
        entity(EntityParts {
            api_key: Some(KEY_ONE),
            spend: Some(2),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
    ];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let key = results[0]
        .breakdown
        .entities
        .get("team-alpha")
        .unwrap()
        .api_key_breakdown
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(key.metrics.spend, amount(2));
}

#[test]
fn rolled_row_keeps_the_existing_key_breakdown() {
    let mut results = vec![day(DAY_ONE)];
    let rows = [
        entity(EntityParts {
            api_key: Some(KEY_ONE),
            spend: Some(1),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
        entity(EntityParts {
            rolled: true,
            spend: Some(9),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
    ];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let bucket = results[0].breakdown.entities.get("team-alpha").unwrap();
    assert_eq!(bucket.metrics.spend, amount(9));
    assert_eq!(
        bucket.api_key_breakdown.get(KEY_ONE).unwrap().metrics.spend,
        amount(1)
    );
}

#[test]
fn key_rows_on_different_dates_land_in_their_buckets() {
    let mut results = vec![day(DAY_ONE), day(DAY_TWO)];
    let rows = [
        entity(EntityParts {
            api_key: Some(KEY_ONE),
            spend: Some(1),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
        entity(EntityParts {
            api_key: Some(KEY_ONE),
            spend: Some(2),
            ..EntityParts::new(Some(DAY_TWO), Some("team-alpha"))
        }),
    ];
    fold_entity_rollups_sync(&mut results, &rows, &BTreeMap::new(), None);
    let spends: Vec<Decimal> = results
        .iter()
        .map(|day| {
            day.breakdown
                .entities
                .get("team-alpha")
                .unwrap()
                .api_key_breakdown
                .get(KEY_ONE)
                .unwrap()
                .metrics
                .spend
        })
        .collect();
    assert_eq!(spends, vec![amount(1), amount(2)]);
}
