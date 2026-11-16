#![cfg(test)]

use betterlite_db::daily_activity_contracts::{AggregatedRows, EntityRollupRow};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::get_daily_activity_aggregated;
use super::levels::{GROUP_DATE, GROUP_DATE_API_KEY, GROUP_GRAND_TOTAL};
use super::support::{
    DAY_ONE, DAY_TWO, EntityParts, KEY_ONE, RepositoryCall, RowParts, amount, empty_aggregated,
    entity, fake, fake_failing_aggregate, fake_failing_metadata, recorded_calls, row, scope,
};

fn entity_total_rows() -> Vec<EntityRollupRow> {
    vec![
        entity(EntityParts {
            rolled: true,
            distinct_api_keys: Some(2),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        }),
        entity(EntityParts {
            rolled: true,
            distinct_api_keys: Some(3),
            ..EntityParts::new(Some(DAY_ONE), Some("team-beta"))
        }),
        entity(EntityParts {
            api_key: Some(KEY_ONE),
            distinct_api_keys: Some(9),
            ..EntityParts::new(Some(DAY_ONE), Some("team-gamma"))
        }),
        entity(EntityParts {
            rolled: true,
            ..EntityParts::new(Some(DAY_ONE), None)
        }),
    ]
}

#[tokio::test]
async fn returns_a_single_page_with_the_requested_limit() {
    let repository = fake(AggregatedRows {
        grouping_rows: vec![
            row(RowParts {
                spend: Some(12),
                ..RowParts::new(GROUP_GRAND_TOTAL, None)
            }),
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
        distinct_api_keys: 3,
    });
    let page = get_daily_activity_aggregated(&repository, &scope(), None, false, 5)
        .await
        .unwrap();
    assert_eq!(page.results.len(), 2);
    assert_eq!(page.metadata.page, 1);
    assert_eq!(page.metadata.total_pages, 1);
    assert!(!page.metadata.has_more);
    assert_eq!(page.metadata.api_key_limit, Some(5));
    assert_eq!(page.metadata.total_api_keys, Some(3));
    assert_eq!(page.metadata.entity_total_api_keys, None);
}

#[tokio::test]
async fn passes_the_scope_and_arguments_to_the_repository() {
    let repository = fake(empty_aggregated());
    get_daily_activity_aggregated(&repository, &scope(), None, true, 7)
        .await
        .unwrap();
    assert_eq!(
        recorded_calls(&repository)[0],
        RepositoryCall::Aggregated {
            start_date: "2024-03-01".to_owned(),
            end_date: "2024-03-31".to_owned(),
            include_entity_breakdown: true,
            api_key_limit: 7,
        }
    );
}

#[tokio::test]
async fn metadata_carries_the_grand_total_metrics() {
    let repository = fake(AggregatedRows {
        grouping_rows: vec![row(RowParts {
            spend: Some(12),
            flat_cost: Some(2),
            prompt_tokens: Some(10),
            completion_tokens: Some(4),
            ..RowParts::new(GROUP_GRAND_TOTAL, None)
        })],
        entity_rows: None,
        distinct_api_keys: 0,
    });
    let page = get_daily_activity_aggregated(&repository, &scope(), None, false, 100)
        .await
        .unwrap();
    assert_eq!(page.metadata.total_spend, amount(12));
    assert_eq!(page.metadata.total_flat_cost, amount(2));
    assert_eq!(page.metadata.total_prompt_tokens, 10);
    assert_eq!(page.metadata.total_completion_tokens, 4);
    assert_eq!(page.metadata.total_tokens, 14);
}

#[tokio::test]
async fn metadata_has_entity_api_key_totals_when_the_breakdown_is_on() {
    let repository = fake(AggregatedRows {
        grouping_rows: vec![row(RowParts {
            spend: Some(5),
            ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
        })],
        entity_rows: Some(entity_total_rows()),
        distinct_api_keys: 3,
    });
    let page = get_daily_activity_aggregated(&repository, &scope(), None, true, 5)
        .await
        .unwrap();
    let mut expected = IndexMap::new();
    expected.insert("team-alpha".to_owned(), 2);
    expected.insert("team-beta".to_owned(), 3);
    assert_eq!(page.metadata.entity_total_api_keys, Some(expected));
}

#[tokio::test]
async fn metadata_has_an_empty_entity_totals_map_without_entity_rows() {
    let repository = fake(empty_aggregated());
    let page = get_daily_activity_aggregated(&repository, &scope(), None, true, 5)
        .await
        .unwrap();
    assert_eq!(page.metadata.entity_total_api_keys, Some(IndexMap::new()));
}

#[tokio::test]
async fn repository_errors_become_a_500() {
    let repository = fake_failing_aggregate();
    let error = get_daily_activity_aggregated(&repository, &scope(), None, false, 5)
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(
        error
            .detail
            .error
            .starts_with("Failed to fetch analytics: ")
    );
}

#[tokio::test]
async fn metadata_errors_become_a_500() {
    let repository = fake_failing_metadata(AggregatedRows {
        grouping_rows: vec![row(RowParts {
            api_key: Some(KEY_ONE),
            ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
        })],
        entity_rows: None,
        distinct_api_keys: 1,
    });
    let error = get_daily_activity_aggregated(&repository, &scope(), None, false, 5)
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[tokio::test]
async fn date_errors_become_a_500_that_names_the_value() {
    let repository = fake(AggregatedRows {
        grouping_rows: vec![row(RowParts::new(GROUP_DATE, Some("not-a-date")))],
        entity_rows: None,
        distinct_api_keys: 0,
    });
    let error = get_daily_activity_aggregated(&repository, &scope(), None, false, 5)
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.detail.error.contains("not-a-date"));
}

#[tokio::test]
async fn folds_entity_rows_with_the_entity_metadata() {
    let mut fields = IndexMap::new();
    fields.insert("team_name".to_owned(), Json::Text("Alpha".to_owned()));
    let mut entity_metadata = super::EntityMetadata::new();
    entity_metadata.insert("team-alpha".to_owned(), fields);
    let repository = fake(AggregatedRows {
        grouping_rows: vec![row(RowParts {
            spend: Some(5),
            ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
        })],
        entity_rows: Some(vec![entity(EntityParts {
            rolled: true,
            spend: Some(5),
            ..EntityParts::new(Some(DAY_ONE), Some("team-alpha"))
        })]),
        distinct_api_keys: 1,
    });
    let page =
        get_daily_activity_aggregated(&repository, &scope(), Some(&entity_metadata), true, 5)
            .await
            .unwrap();
    let bucket = page.results[0]
        .breakdown
        .entities
        .get("team-alpha")
        .unwrap();
    assert_eq!(bucket.metrics.spend, amount(5));
    assert_eq!(
        bucket.metadata.get("team_name"),
        Some(&Json::Text("Alpha".to_owned()))
    );
}
