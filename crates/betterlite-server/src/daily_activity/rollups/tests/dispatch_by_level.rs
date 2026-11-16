#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_analytics::daily_metrics::BreakdownMetrics;
use betterlite_db::daily_activity_contracts::GroupingSetsRow;

use super::dispatch::aggregate_grouping_rows_sync;
use super::levels::{
    GROUP_DATE, GROUP_DATE_API_KEY, GROUP_DATE_ENDPOINT, GROUP_DATE_ENDPOINT_API_KEY,
    GROUP_DATE_MCP, GROUP_DATE_MCP_API_KEY, GROUP_DATE_MODEL, GROUP_DATE_MODEL_API_KEY,
    GROUP_DATE_MODEL_GROUP, GROUP_DATE_MODEL_GROUP_API_KEY, GROUP_DATE_PROVIDER,
    GROUP_DATE_PROVIDER_API_KEY, GROUP_GRAND_TOTAL,
};
use super::support::{
    DAY_ONE, DAY_TWO, KEY_ONE, RowParts, amount, missing_key_metadata, row, zero_metrics,
};
use super::{AggregatedSpendData, DailyActivityAggregateError};

fn aggregate(records: &[GroupingSetsRow]) -> AggregatedSpendData {
    aggregate_grouping_rows_sync(records, &BTreeMap::new()).unwrap()
}

#[test]
fn grand_total_row_sets_totals_without_a_date_bucket() {
    let records = [row(RowParts {
        spend: Some(5),
        prompt_tokens: Some(3),
        completion_tokens: Some(4),
        ..RowParts::new(GROUP_GRAND_TOTAL, None)
    })];
    let aggregated = aggregate(&records);
    assert!(aggregated.results.is_empty());
    assert_eq!(aggregated.totals.spend, amount(5));
    assert_eq!(aggregated.totals.total_tokens, 7);
}

#[test]
fn date_row_sets_the_bucket_metrics() {
    let records = [row(RowParts {
        spend: Some(5),
        ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    assert_eq!(aggregated.results.len(), 1);
    assert_eq!(
        aggregated.results[0].date.format("%Y-%m-%d").to_string(),
        DAY_ONE
    );
    assert_eq!(aggregated.results[0].metrics.spend, amount(5));
    assert_eq!(aggregated.totals, zero_metrics());
}

#[test]
fn two_date_rows_for_one_date_replace_metrics_instead_of_summing() {
    let records = [
        row(RowParts {
            spend: Some(5),
            ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
        }),
        row(RowParts {
            spend: Some(9),
            ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
        }),
    ];
    let aggregated = aggregate(&records);
    assert_eq!(aggregated.results.len(), 1);
    assert_eq!(aggregated.results[0].metrics.spend, amount(9));
}

#[test]
fn model_row_lands_in_the_model_breakdown() {
    let records = [row(RowParts {
        model: Some("gpt-4"),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let day = &aggregated.results[0];
    let model = day.breakdown.models.get("gpt-4").unwrap();
    assert_eq!(model.metrics.spend, amount(5));
    assert!(model.metadata.is_empty());
    assert!(model.api_key_breakdown.is_empty());
    assert_eq!(day.metrics, zero_metrics());
    assert_eq!(aggregated.totals, zero_metrics());
}

#[test]
fn model_group_row_lands_in_the_model_group_breakdown() {
    let records = [row(RowParts {
        model_group: Some("chat"),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL_GROUP, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let group = aggregated.results[0]
        .breakdown
        .model_groups
        .get("chat")
        .unwrap();
    assert_eq!(group.metrics.spend, amount(5));
}

#[test]
fn mcp_row_lands_in_the_mcp_breakdown() {
    let records = [row(RowParts {
        mcp: Some("github__search"),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MCP, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let tool = aggregated.results[0]
        .breakdown
        .mcp_servers
        .get("github__search")
        .unwrap();
    assert_eq!(tool.metrics.spend, amount(5));
}

#[test]
fn endpoint_row_lands_in_the_endpoint_breakdown() {
    let records = [row(RowParts {
        endpoint: Some("/v1/chat/completions"),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_ENDPOINT, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let endpoint = aggregated.results[0]
        .breakdown
        .endpoints
        .get("/v1/chat/completions")
        .unwrap();
    assert_eq!(endpoint.metrics.spend, amount(5));
}

#[test]
fn provider_row_lands_in_the_provider_breakdown() {
    let records = [row(RowParts {
        provider: Some("openai"),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_PROVIDER, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let provider = aggregated.results[0]
        .breakdown
        .providers
        .get("openai")
        .unwrap();
    assert_eq!(provider.metrics.spend, amount(5));
}

#[test]
fn api_key_row_lands_in_the_api_key_breakdown() {
    let records = [row(RowParts {
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let bucket = aggregated.results[0]
        .breakdown
        .api_keys
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(bucket.metrics.spend, amount(5));
    assert_eq!(bucket.metadata, missing_key_metadata());
}

#[test]
fn rolled_model_row_without_a_key_lands_in_the_model_breakdown() {
    let records = [row(RowParts {
        model: Some("gpt-4"),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let model = aggregated.results[0].breakdown.models.get("gpt-4").unwrap();
    assert!(model.api_key_breakdown.is_empty());
}

#[test]
fn model_api_key_row_builds_a_parent_and_key_breakdown() {
    let records = [row(RowParts {
        model: Some("gpt-4"),
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let parent = aggregated.results[0].breakdown.models.get("gpt-4").unwrap();
    assert_eq!(parent.metrics, zero_metrics());
    let key = parent.api_key_breakdown.get(KEY_ONE).unwrap();
    assert_eq!(key.metrics.spend, amount(5));
}

#[test]
fn model_group_api_key_row_builds_a_parent_and_key_breakdown() {
    let records = [row(RowParts {
        model_group: Some("chat"),
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL_GROUP_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let parent = aggregated.results[0]
        .breakdown
        .model_groups
        .get("chat")
        .unwrap();
    assert_eq!(parent.metrics, zero_metrics());
    assert_eq!(
        parent.api_key_breakdown.get(KEY_ONE).unwrap().metrics.spend,
        amount(5)
    );
}

#[test]
fn provider_api_key_row_builds_a_parent_and_key_breakdown() {
    let records = [row(RowParts {
        provider: Some("openai"),
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_PROVIDER_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let parent = aggregated.results[0]
        .breakdown
        .providers
        .get("openai")
        .unwrap();
    assert_eq!(
        parent.api_key_breakdown.get(KEY_ONE).unwrap().metrics.spend,
        amount(5)
    );
}

#[test]
fn mcp_api_key_row_builds_a_parent_and_key_breakdown() {
    let records = [row(RowParts {
        mcp: Some("github__search"),
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MCP_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let parent = aggregated.results[0]
        .breakdown
        .mcp_servers
        .get("github__search")
        .unwrap();
    assert_eq!(
        parent.api_key_breakdown.get(KEY_ONE).unwrap().metrics.spend,
        amount(5)
    );
}

#[test]
fn endpoint_api_key_row_builds_a_parent_and_key_breakdown() {
    let records = [row(RowParts {
        endpoint: Some("/v1/chat/completions"),
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_ENDPOINT_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    let parent = aggregated.results[0]
        .breakdown
        .endpoints
        .get("/v1/chat/completions")
        .unwrap();
    assert_eq!(
        parent.api_key_breakdown.get(KEY_ONE).unwrap().metrics.spend,
        amount(5)
    );
}

#[test]
fn model_row_replaces_parent_metrics_and_keeps_the_key_breakdown() {
    let records = [
        row(RowParts {
            model: Some("gpt-4"),
            api_key: Some(KEY_ONE),
            spend: Some(2),
            ..RowParts::new(GROUP_DATE_MODEL_API_KEY, Some(DAY_ONE))
        }),
        row(RowParts {
            model: Some("gpt-4"),
            spend: Some(7),
            ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
        }),
    ];
    let aggregated = aggregate(&records);
    let parent = aggregated.results[0].breakdown.models.get("gpt-4").unwrap();
    assert_eq!(parent.metrics.spend, amount(7));
    assert!(parent.api_key_breakdown.contains_key(KEY_ONE));
}

#[test]
fn key_breakdown_row_keeps_parent_metrics_assigned_first() {
    let records = [
        row(RowParts {
            model: Some("gpt-4"),
            spend: Some(7),
            ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
        }),
        row(RowParts {
            model: Some("gpt-4"),
            api_key: Some(KEY_ONE),
            spend: Some(2),
            ..RowParts::new(GROUP_DATE_MODEL_API_KEY, Some(DAY_ONE))
        }),
    ];
    let aggregated = aggregate(&records);
    let parent = aggregated.results[0].breakdown.models.get("gpt-4").unwrap();
    assert_eq!(parent.metrics.spend, amount(7));
    assert_eq!(
        parent.api_key_breakdown.get(KEY_ONE).unwrap().metrics.spend,
        amount(2)
    );
}

#[test]
fn unknown_level_creates_an_empty_date_bucket() {
    let records = [row(RowParts {
        spend: Some(5),
        ..RowParts::new(7, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    assert_eq!(aggregated.results.len(), 1);
    assert_eq!(aggregated.results[0].metrics, zero_metrics());
    assert_eq!(aggregated.results[0].breakdown, BreakdownMetrics::default());
    assert_eq!(aggregated.totals, zero_metrics());
}

#[test]
fn dimension_row_without_its_name_creates_only_a_date_bucket() {
    let records = [row(RowParts {
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
    })];
    let aggregated = aggregate(&records);
    assert_eq!(aggregated.results.len(), 1);
    assert!(aggregated.results[0].breakdown.models.is_empty());
    assert_eq!(aggregated.results[0].metrics, zero_metrics());
}

#[test]
fn grand_total_row_without_a_date_does_not_error() {
    let records = [row(RowParts {
        spend: Some(5),
        ..RowParts::new(GROUP_GRAND_TOTAL, None)
    })];
    let aggregated = aggregate(&records);
    assert_eq!(aggregated.totals.spend, amount(5));
}

#[test]
fn date_row_without_a_date_is_an_error() {
    let records = [row(RowParts {
        spend: Some(5),
        ..RowParts::new(GROUP_DATE, None)
    })];
    let error = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap_err();
    assert!(matches!(error, DailyActivityAggregateError::MissingDate));
}

#[test]
fn date_row_with_an_invalid_date_is_an_error() {
    let records = [row(RowParts {
        ..RowParts::new(GROUP_DATE, Some("2024-13-01"))
    })];
    let error = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap_err();
    let DailyActivityAggregateError::Date { value } = error else {
        panic!("expected a date error");
    };
    assert_eq!(value, "2024-13-01");
}

#[test]
fn multiple_dates_produce_one_result_per_date_in_reverse_order() {
    let records = [
        row(RowParts {
            spend: Some(5),
            ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
        }),
        row(RowParts {
            spend: Some(9),
            ..RowParts::new(GROUP_DATE, Some(DAY_TWO))
        }),
    ];
    let aggregated = aggregate(&records);
    assert_eq!(aggregated.results.len(), 2);
    assert_eq!(
        aggregated.results[0].date.format("%Y-%m-%d").to_string(),
        DAY_TWO
    );
    assert_eq!(
        aggregated.results[1].date.format("%Y-%m-%d").to_string(),
        DAY_ONE
    );
}
