use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::records::UsageCounterRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageUnitKey {
    #[serde(rename = "usage_unit")]
    Counter,
    #[serde(rename = "team_id")]
    Team,
    #[serde(rename = "api_key")]
    Key,
    #[serde(rename = "guardrail_id")]
    Guardrail,
}

// a row written before the cost column existed is untracked in full
#[must_use]
pub const fn row_untracked_units(row: &UsageCounterRow) -> i64 {
    match row.cost {
        None => row.units,
        Some(_) => row.untracked_units,
    }
}

// LiteLLM keeps a cost only when it prices at least one unit of the row
#[must_use]
pub const fn row_tracked_cost(row: &UsageCounterRow) -> Option<Decimal> {
    match row.cost {
        Some(cost) if row_untracked_units(row) < row.units => Some(cost),
        Some(_) | None => None,
    }
}

#[must_use]
pub fn sum_counter_units(rows: &[&UsageCounterRow]) -> BTreeMap<String, i64> {
    let mut sums: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        let sum = sums.entry(row.usage_unit.clone()).or_insert(0);
        *sum = sum.saturating_add(row.units);
    }
    sums
}

#[must_use]
pub fn sum_untracked_units(rows: &[&UsageCounterRow]) -> BTreeMap<String, i64> {
    let mut sums: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        let sum = sums.entry(row.usage_unit.clone()).or_insert(0);
        *sum = sum.saturating_add(row_untracked_units(row));
    }
    sums.retain(|_, units| *units != 0);
    sums
}

#[must_use]
pub fn sum_tracked_cost(rows: &[&UsageCounterRow]) -> Option<Decimal> {
    let mut total: Option<Decimal> = None;
    for row in rows {
        let Some(cost) = row_tracked_cost(row) else {
            continue;
        };
        total = Some(match total {
            Some(sum) => sum.saturating_add(cost),
            None => cost,
        });
    }
    total
}

#[must_use]
pub fn by<T, R>(rows: &[UsageCounterRow], key: UsageUnitKey, reduce: R) -> BTreeMap<String, T>
where
    R: Fn(&[&UsageCounterRow]) -> T,
{
    let mut groups: BTreeMap<String, Vec<&UsageCounterRow>> = BTreeMap::new();
    for row in rows {
        let group = match key {
            UsageUnitKey::Counter => &row.usage_unit,
            UsageUnitKey::Team => &row.team_id,
            UsageUnitKey::Key => &row.api_key,
            UsageUnitKey::Guardrail => &row.guardrail_id,
        };
        groups.entry(group.clone()).or_default().push(row);
    }
    groups
        .into_iter()
        .map(|(group, group_rows)| (group, reduce(&group_rows)))
        .collect()
}

#[must_use]
pub fn first_match<'map, T>(
    lookup_keys: &[&str],
    mapping: &'map BTreeMap<String, T>,
    default: &'map T,
) -> &'map T {
    for key in lookup_keys {
        if let Some(value) = mapping.get(*key) {
            return value;
        }
    }
    default
}

mod tests;
