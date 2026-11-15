use std::collections::BTreeMap;
use std::hash::Hash;
use std::mem;

use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::payload::{
    billed_cost_by_unit, ids_by_name, information_entries, payload_stamp, resolved_id, unit_count,
    unit_guardrail_id,
};
use super::{
    GuardrailAction, GuardrailCounts, GuardrailSpendLog, MAX_PENDING_ROWS, MetricsKey, RollupRow,
    UsageUnitIncrement, UsageUnitKey, guardrail_status_to_action,
};

#[must_use]
pub(crate) fn fresh_metrics(
    payloads: &[GuardrailSpendLog],
) -> IndexMap<MetricsKey, GuardrailCounts> {
    let mut metrics: IndexMap<MetricsKey, GuardrailCounts> = IndexMap::new();
    for payload in payloads {
        let Some(stamp) = payload_stamp(payload) else {
            continue;
        };
        let entries = information_entries(payload);
        let ids = ids_by_name(entries);
        for entry in entries {
            let Some(guardrail_id) = resolved_id(entry, &ids) else {
                continue;
            };
            let action = guardrail_status_to_action(entry.guardrail_status.as_deref());
            if action == GuardrailAction::NotRun {
                continue;
            }
            let key = MetricsKey {
                guardrail_id,
                date: stamp.date.clone(),
            };
            let counts = metrics.entry(key).or_default();
            counts.requests_evaluated = counts.requests_evaluated.saturating_add(1);
            let counter = match action {
                GuardrailAction::Passed => &mut counts.passed_count,
                GuardrailAction::Blocked => &mut counts.blocked_count,
                GuardrailAction::Flagged => &mut counts.flagged_count,
                GuardrailAction::NotRun => continue,
            };
            *counter = counter.saturating_add(1);
        }
    }
    metrics
}

#[must_use]
pub(crate) fn fresh_units(
    payloads: &[GuardrailSpendLog],
) -> IndexMap<UsageUnitKey, UsageUnitIncrement> {
    let mut units: BTreeMap<UsageUnitKey, UsageUnitIncrement> = BTreeMap::new();
    for payload in payloads {
        let Some(stamp) = payload_stamp(payload) else {
            continue;
        };
        let team_id = payload.team_id.clone().unwrap_or_default();
        let api_key = payload.api_key.clone().unwrap_or_default();
        for entry in information_entries(payload) {
            let Some(guardrail_id) = unit_guardrail_id(entry) else {
                continue;
            };
            let Some(usage) = entry.guardrail_usage.as_ref() else {
                continue;
            };
            let costs = billed_cost_by_unit(entry);
            for (unit_name, value) in usage {
                let Some(count) = unit_count(value) else {
                    continue;
                };
                let key = UsageUnitKey {
                    guardrail_id: guardrail_id.clone(),
                    date: stamp.date.clone(),
                    team_id: team_id.clone(),
                    api_key: api_key.clone(),
                    usage_unit: unit_name.clone(),
                };
                let cost = costs
                    .as_ref()
                    .and_then(|costs| costs.get(unit_name))
                    .copied()
                    .flatten();
                units.entry(key).or_default().add_usage(count, cost);
            }
        }
    }
    units.into_iter().collect()
}

impl UsageUnitIncrement {
    fn add_usage(&mut self, count: i64, cost: Option<Decimal>) {
        self.units = self.units.saturating_add(count);
        match cost {
            Some(cost) => self.cost = self.cost.checked_add(cost).unwrap_or(Decimal::MAX),
            None => self.untracked_units = self.untracked_units.saturating_add(count),
        }
    }
}

#[must_use]
pub(crate) const fn summed_counts(
    left: GuardrailCounts,
    right: GuardrailCounts,
) -> GuardrailCounts {
    GuardrailCounts {
        requests_evaluated: left
            .requests_evaluated
            .saturating_add(right.requests_evaluated),
        passed_count: left.passed_count.saturating_add(right.passed_count),
        blocked_count: left.blocked_count.saturating_add(right.blocked_count),
        flagged_count: left.flagged_count.saturating_add(right.flagged_count),
    }
}

#[must_use]
pub(crate) fn summed_units(
    left: UsageUnitIncrement,
    right: UsageUnitIncrement,
) -> UsageUnitIncrement {
    UsageUnitIncrement {
        units: left.units.saturating_add(right.units),
        cost: left.cost.checked_add(right.cost).unwrap_or(Decimal::MAX),
        untracked_units: left.untracked_units.saturating_add(right.untracked_units),
    }
}

// Python keeps the queued keys first, then appends keys only the fresh rows have
#[must_use]
pub(crate) fn merged_rows<Key, Value>(
    base: IndexMap<Key, Value>,
    extra: IndexMap<Key, Value>,
    sum: fn(Value, Value) -> Value,
) -> IndexMap<Key, Value>
where
    Key: Eq + Hash,
    Value: Default,
{
    let mut merged = base;
    for (key, value) in extra {
        let current = merged.entry(key).or_default();
        let previous = mem::take(current);
        *current = sum(previous, value);
    }
    merged
}

// LiteLLM keeps the newest rows and drops the oldest past the cap, with a warning that never fails the flush
#[must_use]
pub(crate) fn capped<Key, Value>(rows: IndexMap<Key, Value>, label: &str) -> IndexMap<Key, Value>
where
    Key: Eq + Hash,
{
    let excess = rows.len().saturating_sub(MAX_PENDING_ROWS);
    if rows.len() > MAX_PENDING_ROWS {
        tracing::warn!(
            "guardrail usage: pending {label} exceeds {MAX_PENDING_ROWS} rows; dropping the {excess} oldest"
        );
    }
    rows.into_iter().skip(excess).collect()
}

#[must_use]
pub(crate) fn split(
    rows: Vec<RollupRow>,
) -> (
    IndexMap<MetricsKey, GuardrailCounts>,
    IndexMap<UsageUnitKey, UsageUnitIncrement>,
) {
    let mut metrics = IndexMap::new();
    let mut units = IndexMap::new();
    for row in rows {
        match row {
            RollupRow::Metrics { key, counts } => {
                metrics.insert(key, counts);
            }
            RollupRow::Units { key, increment } => {
                units.insert(key, increment);
            }
        }
    }
    (metrics, units)
}
