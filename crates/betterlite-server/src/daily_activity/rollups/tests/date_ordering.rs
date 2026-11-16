#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::daily_activity_contracts::AggregatedRows;
use chrono::NaiveDate;

use super::dispatch::aggregate_grouping_rows_sync;
use super::get_daily_activity_aggregated;
use super::levels::{GROUP_DATE, GROUP_GRAND_TOTAL};
use super::support::{DAY_ONE, DAY_TWO, RowParts, amount, day, fake, row, scope, zero_metrics};

const DAY_THREE: &str = "2024-03-07";

#[test]
fn results_are_sorted_by_date_descending() {
    let records = vec![
        row(RowParts::new(GROUP_DATE, Some(DAY_TWO))),
        row(RowParts::new(GROUP_DATE, Some(DAY_ONE))),
        row(RowParts::new(GROUP_DATE, Some(DAY_THREE))),
    ];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let dates: Vec<NaiveDate> = aggregated.results.iter().map(|day| day.date).collect();
    assert_eq!(
        dates,
        vec![day(DAY_THREE).date, day(DAY_TWO).date, day(DAY_ONE).date]
    );
}

#[test]
fn a_grand_total_between_date_rows_sets_the_totals() {
    let records = vec![
        row(RowParts::new(GROUP_DATE, Some(DAY_ONE))),
        row(RowParts {
            spend: Some(9),
            prompt_tokens: Some(3),
            ..RowParts::new(GROUP_GRAND_TOTAL, None)
        }),
        row(RowParts::new(GROUP_DATE, Some(DAY_TWO))),
    ];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert_eq!(aggregated.totals.spend, amount(9));
    assert_eq!(aggregated.totals.prompt_tokens, 3);
    assert_eq!(aggregated.results.len(), 2);
    assert_eq!(aggregated.results[0].date, day(DAY_TWO).date);
}

#[test]
fn a_grand_total_row_creates_no_date_group() {
    let records = vec![row(RowParts {
        spend: Some(12),
        ..RowParts::new(GROUP_GRAND_TOTAL, None)
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert!(aggregated.results.is_empty());
    assert_eq!(aggregated.totals.spend, amount(12));
}

#[test]
fn a_date_row_without_metrics_stays_zero() {
    let records = vec![row(RowParts::new(GROUP_DATE, Some(DAY_ONE)))];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert_eq!(aggregated.results.len(), 1);
    assert_eq!(aggregated.results[0].metrics, zero_metrics());
}

#[tokio::test]
async fn response_results_are_sorted_by_date_descending() {
    let repository = fake(AggregatedRows {
        grouping_rows: vec![
            row(RowParts {
                spend: Some(5),
                ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
            }),
            row(RowParts {
                spend: Some(7),
                ..RowParts::new(GROUP_DATE, Some(DAY_TWO))
            }),
        ],
        entity_rows: None,
        distinct_api_keys: 0,
    });
    let page = get_daily_activity_aggregated(&repository, &scope(), None, false, 5)
        .await
        .unwrap();
    assert_eq!(page.results.len(), 2);
    assert_eq!(page.results[0].date, day(DAY_TWO).date);
    assert_eq!(page.results[1].date, day(DAY_ONE).date);
}
