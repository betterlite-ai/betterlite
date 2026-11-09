use std::ops::Add as _;
use std::time::Duration;

use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::NewRelicMetricsError;
use super::records::NewRelicMetricRecord;
use super::wire::{
    ATTRIBUTE_MAX_LEN, CountValue, METRIC_COMPLETION_TOKENS, METRIC_COST_USD, METRIC_PROMPT_TOKENS,
    METRIC_REQUEST_DURATION_MS, METRIC_REQUESTS, METRIC_TEAM_MAX_BUDGET,
    METRIC_TEAM_REMAINING_BUDGET, METRIC_TOTAL_TOKENS, MetricAttributes, NewRelicMetric,
    NewRelicMetricCommon, NewRelicMetricEnvelope, NewRelicSummaryValue,
};

#[derive(Clone, Eq, Hash, PartialEq)]
struct BucketKey {
    team_id: String,
    team_alias: String,
    model_group: String,
    model: String,
    custom_llm_provider: String,
    status: String,
}

struct BucketTotals {
    cost: Decimal,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    duration_sum: f64,
    duration_min: f64,
    duration_max: f64,
}

fn metric_attributes(pairs: &[(&str, &str)]) -> MetricAttributes {
    let mut attributes = MetricAttributes::new();
    for (key, value) in pairs {
        if !value.is_empty() {
            attributes.insert(
                String::from(*key),
                value.chars().take(ATTRIBUTE_MAX_LEN).collect(),
            );
        }
    }
    attributes
}

fn bucket_totals(records: &[&NewRelicMetricRecord]) -> Result<BucketTotals, NewRelicMetricsError> {
    let mut totals = BucketTotals {
        cost: Decimal::ZERO,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        duration_sum: 0.0_f64,
        duration_min: f64::INFINITY,
        duration_max: f64::NEG_INFINITY,
    };
    for record in records {
        totals.cost = totals
            .cost
            .checked_add(record.response_cost)
            .ok_or(NewRelicMetricsError::CostSum)?;
        totals.prompt_tokens = totals
            .prompt_tokens
            .checked_add(record.prompt_tokens)
            .ok_or(NewRelicMetricsError::TokenSum)?;
        totals.completion_tokens = totals
            .completion_tokens
            .checked_add(record.completion_tokens)
            .ok_or(NewRelicMetricsError::TokenSum)?;
        totals.total_tokens = totals
            .total_tokens
            .checked_add(record.total_tokens)
            .ok_or(NewRelicMetricsError::TokenSum)?;
        totals.duration_sum = totals.duration_sum.add(record.duration_ms);
        totals.duration_min = totals.duration_min.min(record.duration_ms);
        totals.duration_max = totals.duration_max.max(record.duration_ms);
    }
    Ok(totals)
}

fn count_metrics(
    record_count: usize,
    totals: &BucketTotals,
    attributes: &MetricAttributes,
) -> Vec<NewRelicMetric> {
    let values = [
        (
            METRIC_REQUESTS,
            CountValue::Float(Decimal::from(record_count).as_f64()),
        ),
        (METRIC_COST_USD, CountValue::Decimal(totals.cost)),
        (
            METRIC_PROMPT_TOKENS,
            CountValue::Float(Decimal::from(totals.prompt_tokens).as_f64()),
        ),
        (
            METRIC_COMPLETION_TOKENS,
            CountValue::Float(Decimal::from(totals.completion_tokens).as_f64()),
        ),
        (
            METRIC_TOTAL_TOKENS,
            CountValue::Float(Decimal::from(totals.total_tokens).as_f64()),
        ),
    ];
    values
        .into_iter()
        .map(|(name, value)| NewRelicMetric::Count {
            name,
            value,
            attributes: attributes.clone(),
        })
        .collect()
}

fn bucket_metrics(
    records: &[&NewRelicMetricRecord],
) -> Result<Vec<NewRelicMetric>, NewRelicMetricsError> {
    let Some(first) = records.first() else {
        return Ok(Vec::new());
    };
    let attributes = metric_attributes(&[
        ("team_id", first.team_id.as_str()),
        ("team_alias", first.team_alias.as_str()),
        ("model_group", first.model_group.as_str()),
        ("model", first.model.as_str()),
        ("custom_llm_provider", first.custom_llm_provider.as_str()),
        ("status", first.status.as_str()),
    ]);
    let totals = bucket_totals(records)?;
    let mut metrics = count_metrics(records.len(), &totals, &attributes);
    metrics.push(NewRelicMetric::Summary {
        name: METRIC_REQUEST_DURATION_MS,
        value: NewRelicSummaryValue {
            count: records.len(),
            sum: totals.duration_sum,
            min: totals.duration_min,
            max: totals.duration_max,
        },
        attributes,
    });
    Ok(metrics)
}

fn team_budget_gauges(
    record: &NewRelicMetricRecord,
) -> Result<Vec<NewRelicMetric>, NewRelicMetricsError> {
    let Some(max_budget) = record.team_max_budget else {
        return Ok(Vec::new());
    };
    let attributes = metric_attributes(&[
        ("team_id", record.team_id.as_str()),
        ("team_alias", record.team_alias.as_str()),
    ]);
    let spend = record.team_spend.unwrap_or(Decimal::ZERO);
    let remaining = max_budget
        .checked_sub(spend)
        .and_then(|value| value.checked_sub(record.response_cost))
        .ok_or(NewRelicMetricsError::BudgetSum)?;
    Ok(vec![
        NewRelicMetric::Gauge {
            name: METRIC_TEAM_MAX_BUDGET,
            value: max_budget,
            attributes: attributes.clone(),
        },
        NewRelicMetric::Gauge {
            name: METRIC_TEAM_REMAINING_BUDGET,
            value: remaining,
            attributes,
        },
    ])
}

fn latest_records_by_team(records: &[NewRelicMetricRecord]) -> Vec<&NewRelicMetricRecord> {
    let mut latest_by_team: IndexMap<&str, &NewRelicMetricRecord> = IndexMap::new();
    for record in records {
        if !record.team_id.is_empty() {
            latest_by_team.insert(record.team_id.as_str(), record);
        }
    }
    latest_by_team.into_values().collect()
}

pub fn build_metric_payload(
    records: &[NewRelicMetricRecord],
    window_start: Duration,
    now: Duration,
) -> Result<NewRelicMetricEnvelope, NewRelicMetricsError> {
    let timestamp =
        u64::try_from(window_start.as_millis()).map_err(NewRelicMetricsError::WindowStart)?;
    let interval = u64::try_from(now.saturating_sub(window_start).as_millis())
        .map_err(NewRelicMetricsError::WindowLength)?
        .max(1);
    let mut buckets: IndexMap<BucketKey, Vec<&NewRelicMetricRecord>> = IndexMap::new();
    for record in records {
        buckets
            .entry(BucketKey {
                team_id: record.team_id.clone(),
                team_alias: record.team_alias.clone(),
                model_group: record.model_group.clone(),
                model: record.model.clone(),
                custom_llm_provider: record.custom_llm_provider.clone(),
                status: record.status.clone(),
            })
            .or_default()
            .push(record);
    }
    let mut metrics = Vec::new();
    for bucket in buckets.values() {
        metrics.extend(bucket_metrics(bucket)?);
    }
    for record in latest_records_by_team(records) {
        metrics.extend(team_budget_gauges(record)?);
    }
    Ok(NewRelicMetricEnvelope {
        common: NewRelicMetricCommon {
            timestamp,
            interval_ms: interval,
        },
        metrics,
    })
}
