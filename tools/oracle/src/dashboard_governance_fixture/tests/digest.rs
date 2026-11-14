use betterlite_postgres_fixture::dashboard_governance::DashboardGovernanceSeed;
use rust_decimal::Decimal;

fn decimal(value: Option<&Decimal>) -> Option<String> {
    value.map(ToString::to_string)
}

pub(super) fn measurements(seed: &DashboardGovernanceSeed) -> Vec<String> {
    let mut lines = Vec::new();
    for metric in &seed.metrics {
        lines.push(format!(
            "metric|{}|{}|{}|{}|{}|{}|{:?}|{:?}",
            metric.guardrail_id,
            metric.date,
            metric.requests_evaluated,
            metric.passed_count,
            metric.blocked_count,
            metric.flagged_count,
            decimal(metric.avg_score.as_ref()),
            decimal(metric.avg_latency_ms.as_ref()),
        ));
    }
    for unit in &seed.usage_units {
        lines.push(format!(
            "unit|{}|{}|{}|{:?}|{}|{}|{:?}|{}",
            unit.guardrail_id,
            unit.date,
            unit.team_id,
            unit.key_index,
            unit.usage_unit,
            unit.units,
            decimal(unit.cost.as_ref()),
            unit.untracked_units,
        ));
    }
    lines
}
