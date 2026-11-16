use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::guardrails::usage_projections::aggregation::{
    UsageUnitKey, by, row_tracked_cost, row_untracked_units, sum_counter_units, sum_tracked_cost,
    sum_untracked_units,
};
use betterlite_server::guardrails::usage_projections::metrics::{
    aggregate_daily_metrics, chart_from_metrics, prev_fail_rates,
};
use betterlite_server::guardrails::usage_projections::overview::{
    UsageAggregates, guardrail_overview_rows, policy_overview_rows,
};
use betterlite_server::guardrails::usage_projections::records::{
    GuardrailRecord, PolicyRecord, UsageCounterRow, UsageMetricRow,
};
use betterlite_server::guardrails::usage_projections::status::{
    status_from_fail_rate, trend_from_comparison,
};
use betterlite_server::guardrails::usage_projections::window::resolve_usage_window;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ResolveWindow { cases: Vec<WindowCase> },
    Thresholds { cases: Vec<ThresholdCase> },
    DailyMetrics { cases: Vec<DailyCase> },
    RowSums { cases: Vec<SumCase> },
    RowParts { cases: Vec<RowsCase> },
    GroupBy { cases: Vec<GroupCase> },
    GuardrailOverview { cases: Vec<GuardrailCase> },
    PolicyOverview { cases: Vec<PolicyCase> },
}

#[derive(Deserialize)]
pub(crate) struct WindowCase {
    start_date: Option<String>,
    end_date: Option<String>,
    now: DateTime<Utc>,
}

#[derive(Deserialize)]
pub(crate) struct ThresholdCase {
    fail_rate: f64,
    previous: f64,
}

#[derive(Deserialize)]
pub(crate) struct DailyCase {
    reduce: DailyReduce,
    rows: Vec<UsageMetricRow>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DailyReduce {
    Aggregate,
    Previous,
    Chart,
}

#[derive(Deserialize)]
pub(crate) struct SumCase {
    reduce: SumReduce,
    rows: Vec<UsageCounterRow>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SumReduce {
    Units,
    Untracked,
    Cost,
}

#[derive(Deserialize)]
pub(crate) struct RowsCase {
    rows: Vec<UsageCounterRow>,
}

#[derive(Deserialize)]
pub(crate) struct GroupCase {
    key: UsageUnitKey,
    reduce: SumReduce,
    rows: Vec<UsageCounterRow>,
}

#[derive(Deserialize)]
pub(crate) struct GuardrailCase {
    records: Vec<GuardrailRecord>,
    aggregates: UsageAggregates,
}

#[derive(Deserialize)]
pub(crate) struct PolicyCase {
    records: Vec<PolicyRecord>,
    aggregates: UsageAggregates,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::ResolveWindow { cases } => resolve_window_cases(cases),
            Input::Thresholds { cases } => threshold_cases(cases),
            Input::DailyMetrics { cases } => daily_cases(cases),
            Input::RowSums { cases } => row_sum_cases(cases),
            Input::RowParts { cases } => row_part_cases(cases),
            Input::GroupBy { cases } => group_cases(cases),
            Input::GuardrailOverview { cases } => guardrail_cases(cases),
            Input::PolicyOverview { cases } => policy_cases(cases),
        }
    }
}

fn resolve_window_cases(cases: &[WindowCase]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        let window = resolve_usage_window(
            case.start_date.as_deref(),
            case.end_date.as_deref(),
            case.now,
        );
        let value = match window {
            Ok(window) => json!({"start": window.start, "end": window.end}),
            Err(error) => json!({"status": error.status_code().as_u16()}),
        };
        results.push(value);
    }
    Ok(json!({"results": results}))
}

fn threshold_cases(cases: &[ThresholdCase]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        let value = json!({
            "status": status_from_fail_rate(case.fail_rate),
            "trend": trend_from_comparison(case.fail_rate, case.previous),
        });
        results.push(value);
    }
    Ok(json!({"results": results}))
}

fn daily_cases(cases: &[DailyCase]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        let value = match case.reduce {
            DailyReduce::Aggregate => json!(aggregate_daily_metrics(&case.rows)),
            DailyReduce::Previous => json!(prev_fail_rates(&case.rows)),
            DailyReduce::Chart => json!(chart_from_metrics(&case.rows)),
        };
        results.push(value);
    }
    Ok(json!({"results": results}))
}

fn row_sum_cases(cases: &[SumCase]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        let rows: Vec<&UsageCounterRow> = case.rows.iter().collect();
        let value = match case.reduce {
            SumReduce::Units => json!(sum_counter_units(&rows)),
            SumReduce::Untracked => json!(sum_untracked_units(&rows)),
            SumReduce::Cost => json!(sum_tracked_cost(&rows).map(|cost| cost.as_f64())),
        };
        results.push(value);
    }
    Ok(json!({"results": results}))
}

fn row_part_cases(cases: &[RowsCase]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        let parts: Vec<Value> = case
            .rows
            .iter()
            .map(|row| {
                json!({
                    "untracked": row_untracked_units(row),
                    "cost": row_tracked_cost(row).map(|cost| cost.as_f64()),
                })
            })
            .collect();
        results.push(json!(parts));
    }
    Ok(json!({"results": results}))
}

fn group_cases(cases: &[GroupCase]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        let value = match case.reduce {
            SumReduce::Units => json!(by(&case.rows, case.key, sum_counter_units)),
            SumReduce::Untracked => json!(by(&case.rows, case.key, sum_untracked_units)),
            SumReduce::Cost => {
                let groups: BTreeMap<String, Option<f64>> =
                    by(&case.rows, case.key, sum_tracked_cost)
                        .into_iter()
                        .map(|(group, total)| (group, total.map(|cost| cost.as_f64())))
                        .collect();
                json!(groups)
            }
        };
        results.push(value);
    }
    Ok(json!({"results": results}))
}

fn guardrail_cases(cases: &[GuardrailCase]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        results.push(json!(guardrail_overview_rows(
            &case.records,
            &case.aggregates
        )));
    }
    Ok(json!({"results": results}))
}

fn policy_cases(cases: &[PolicyCase]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        results.push(json!(policy_overview_rows(&case.records, &case.aggregates)));
    }
    Ok(json!({"results": results}))
}

mod tests;
