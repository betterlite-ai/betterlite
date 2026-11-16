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
        let results = match input {
            Input::ResolveWindow { cases } => answers(cases, window_answer),
            Input::Thresholds { cases } => answers(cases, threshold_answer),
            Input::DailyMetrics { cases } => answers(cases, daily_answer),
            Input::RowSums { cases } => answers(cases, row_sum_answer),
            Input::RowParts { cases } => answers(cases, row_part_answer),
            Input::GroupBy { cases } => answers(cases, group_answer),
            Input::GuardrailOverview { cases } => answers(cases, |case| {
                json!(guardrail_overview_rows(&case.records, &case.aggregates))
            }),
            Input::PolicyOverview { cases } => answers(cases, |case| {
                json!(policy_overview_rows(&case.records, &case.aggregates))
            }),
        };
        Ok(json!({"results": results}))
    }
}

fn answers<C, A>(cases: &[C], answer: A) -> Vec<Value>
where
    A: Fn(&C) -> Value,
{
    let mut values = Vec::with_capacity(cases.len());
    for case in cases {
        values.push(answer(case));
    }
    values
}

fn window_answer(case: &WindowCase) -> Value {
    let start = case.start_date.as_deref();
    let end = case.end_date.as_deref();
    resolve_usage_window(start, end, case.now).map_or_else(
        |error| json!({"status": error.status_code().as_u16()}),
        |window| json!({"start": window.start, "end": window.end}),
    )
}

fn threshold_answer(case: &ThresholdCase) -> Value {
    json!({
        "status": status_from_fail_rate(case.fail_rate),
        "trend": trend_from_comparison(case.fail_rate, case.previous),
    })
}

fn daily_answer(case: &DailyCase) -> Value {
    match case.reduce {
        DailyReduce::Aggregate => json!(aggregate_daily_metrics(&case.rows)),
        DailyReduce::Previous => json!(prev_fail_rates(&case.rows)),
        DailyReduce::Chart => json!(chart_from_metrics(&case.rows)),
    }
}

fn row_sum_answer(case: &SumCase) -> Value {
    let rows: Vec<&UsageCounterRow> = case.rows.iter().collect();
    match case.reduce {
        SumReduce::Units => json!(sum_counter_units(&rows, |row| row.units)),
        SumReduce::Untracked => json!(sum_untracked_units(&rows)),
        SumReduce::Cost => json!(sum_tracked_cost(&rows).map(|cost| cost.as_f64())),
    }
}

fn row_part_answer(case: &RowsCase) -> Value {
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
    json!(parts)
}

fn group_answer(case: &GroupCase) -> Value {
    match case.reduce {
        SumReduce::Units => json!(by(&case.rows, case.key, |group| sum_counter_units(
            group,
            |row| row.units
        ))),
        SumReduce::Untracked => json!(by(&case.rows, case.key, sum_untracked_units)),
        SumReduce::Cost => json!(grouped_costs(&case.rows, case.key)),
    }
}

fn grouped_costs(rows: &[UsageCounterRow], key: UsageUnitKey) -> BTreeMap<String, Option<f64>> {
    by(rows, key, sum_tracked_cost)
        .into_iter()
        .map(|(group, total)| (group, total.map(|cost| cost.as_f64())))
        .collect()
}

mod tests;
