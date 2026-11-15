use std::collections::BTreeMap;

use betterlite_models::tool_spend::ToolUsageTransaction;
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDayRollup {
    pub date: String,
    pub tool_name: String,
    pub spend: Decimal,
    pub total_tokens: i64,
    pub request_count: i64,
}

#[derive(Debug, Default)]
struct DayToolTotal {
    spend: Decimal,
    total_tokens: i64,
    request_count: i64,
}

// One rollup per date and tool in (date, tool) order, each occurrence counted once for request_count
#[must_use]
pub fn daily_rollups(transactions: &[ToolUsageTransaction]) -> Vec<ToolDayRollup> {
    let mut grouped: BTreeMap<(&str, &str), DayToolTotal> = BTreeMap::new();
    for transaction in transactions {
        for tool_name in &transaction.tool_names {
            let total = grouped
                .entry((transaction.date.as_str(), tool_name.as_str()))
                .or_default();
            total.spend = total
                .spend
                .checked_add(transaction.spend)
                .unwrap_or(Decimal::MAX);
            total.total_tokens = total.total_tokens.saturating_add(transaction.total_tokens);
            total.request_count = total.request_count.saturating_add(1);
        }
    }
    grouped
        .into_iter()
        .map(|((date, tool_name), total)| ToolDayRollup {
            date: date.to_owned(),
            tool_name: tool_name.to_owned(),
            spend: total.spend,
            total_tokens: total.total_tokens,
            request_count: total.request_count,
        })
        .collect()
}
