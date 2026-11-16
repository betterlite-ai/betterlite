#![cfg(test)]

use std::collections::BTreeSet;
use std::thread::current;

use betterlite_db::daily_activity_contracts::AggregatedRows;

use super::DailyActivityAggregateError;
use super::dispatch::{aggregate_grouping_rows, aggregate_grouping_rows_sync};
use super::levels::{GROUP_DATE, GROUP_DATE_API_KEY, GROUP_DATE_MODEL};
use super::support::{
    DAY_ONE, DAY_TWO, KEY_ONE, KEY_TWO, RepositoryCall, RowParts, SENTINEL, at, empty_aggregated,
    fake, fake_failing_metadata, key_map, key_metadata_for, key_row, recorded_calls, row, window,
    zero_metrics,
};

#[tokio::test]
async fn wrapper_aggregates_the_grouping_rows() {
    let records = vec![
        row(RowParts {
            spend: Some(5),
            ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
        }),
        row(RowParts {
            spend: Some(9),
            ..RowParts::new(GROUP_DATE, Some(DAY_TWO))
        }),
    ];
    let repository = fake(AggregatedRows {
        grouping_rows: records.clone(),
        entity_rows: None,
        distinct_api_keys: 0,
    });
    let aggregated = aggregate_grouping_rows(&repository, records).await.unwrap();
    assert_eq!(aggregated.results.len(), 2);
    assert_eq!(aggregated.totals, zero_metrics());
}

#[tokio::test]
async fn wrapper_matches_the_sync_function() {
    let records = vec![row(RowParts {
        model: Some("gpt-4"),
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
    })];
    let metadata = key_map(&[key_row(KEY_ONE, true)]);
    let expected = aggregate_grouping_rows_sync(&records, &metadata).unwrap();
    let mut repository = fake(empty_aggregated());
    repository.metadata = metadata;
    let aggregated = aggregate_grouping_rows(&repository, records).await.unwrap();
    assert_eq!(aggregated.results, expected.results);
    assert_eq!(aggregated.totals, expected.totals);
}

#[tokio::test]
async fn wrapper_fetches_metadata_for_non_sentinel_keys() {
    let records = vec![
        row(RowParts {
            api_key: Some(KEY_ONE),
            ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
        }),
        row(RowParts {
            api_key: Some(SENTINEL),
            ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
        }),
    ];
    let repository = fake(empty_aggregated());
    aggregate_grouping_rows(&repository, records).await.unwrap();
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
async fn wrapper_skips_metadata_when_only_sentinels_are_present() {
    let records = vec![row(RowParts {
        api_key: Some(SENTINEL),
        ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
    })];
    let repository = fake(empty_aggregated());
    aggregate_grouping_rows(&repository, records).await.unwrap();
    assert!(recorded_calls(&repository).is_empty());
}

#[tokio::test]
async fn wrapper_skips_metadata_when_there_are_no_api_keys() {
    let records = vec![row(RowParts {
        ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
    })];
    let repository = fake(empty_aggregated());
    aggregate_grouping_rows(&repository, records).await.unwrap();
    assert!(recorded_calls(&repository).is_empty());
}

#[tokio::test]
async fn wrapper_window_spans_the_earliest_and_latest_dates() {
    let records = vec![
        row(RowParts {
            api_key: Some(KEY_ONE),
            ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_TWO))
        }),
        row(RowParts {
            api_key: Some(KEY_TWO),
            ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
        }),
    ];
    let repository = fake(empty_aggregated());
    aggregate_grouping_rows(&repository, records).await.unwrap();
    let calls = recorded_calls(&repository);
    assert_eq!(
        calls[0],
        RepositoryCall::Metadata {
            api_keys: BTreeSet::from([KEY_ONE.to_owned(), KEY_TWO.to_owned()]),
            window: Some(window(at(2024, 3, 4), at(2024, 3, 8))),
        }
    );
}

#[tokio::test]
async fn wrapper_attaches_key_metadata_from_the_repository() {
    let records = vec![row(RowParts {
        api_key: Some(KEY_ONE),
        ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
    })];
    let mut repository = fake(empty_aggregated());
    repository.metadata = key_map(&[key_row(KEY_ONE, true)]);
    let aggregated = aggregate_grouping_rows(&repository, records).await.unwrap();
    let bucket = aggregated.results[0]
        .breakdown
        .api_keys
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(bucket.metadata, key_metadata_for(KEY_ONE, true));
}

#[tokio::test]
async fn wrapper_propagates_metadata_errors() {
    let records = vec![row(RowParts {
        api_key: Some(KEY_ONE),
        ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
    })];
    let repository = fake_failing_metadata(empty_aggregated());
    let error = aggregate_grouping_rows(&repository, records)
        .await
        .unwrap_err();
    assert!(matches!(error, DailyActivityAggregateError::Repository(_)));
}

#[tokio::test]
async fn wrapper_returns_worker_errors() {
    let records = vec![row(RowParts {
        ..RowParts::new(GROUP_DATE, None)
    })];
    let repository = fake(empty_aggregated());
    let error = aggregate_grouping_rows(&repository, records)
        .await
        .unwrap_err();
    assert!(matches!(error, DailyActivityAggregateError::MissingDate));
}

#[tokio::test]
async fn run_worker_runs_the_operation_on_a_worker_thread() {
    let caller = current().id();
    let worker = super::dispatch::run_worker(move || current().id())
        .await
        .unwrap();
    assert_ne!(worker, caller);
}
