use betterlite_db::spend_queues::entity::SpendAmount;
use chrono::NaiveDate;
use indexmap::IndexMap;

use crate::spend::log_payload::daily_summary::SummaryValue;

// view_spend_logs pads every day after the last covered day through end_date with an empty summary
#[must_use]
pub fn pad_summary(
    items: Vec<IndexMap<String, SummaryValue>>,
    end_date: NaiveDate,
) -> Vec<IndexMap<String, SummaryValue>> {
    let Some(last_day) = items.iter().rev().find_map(summary_day) else {
        return items;
    };
    let mut padded = items;
    for day in last_day
        .iter_days()
        .skip(1)
        .take_while(|day| *day <= end_date)
    {
        let mut item = IndexMap::new();
        item.insert(String::from("startTime"), SummaryValue::Day(day));
        item.insert(
            String::from("spend"),
            SummaryValue::Spend(SpendAmount::ZERO),
        );
        item.insert(
            String::from("users"),
            SummaryValue::Breakdown(IndexMap::new()),
        );
        item.insert(
            String::from("models"),
            SummaryValue::Breakdown(IndexMap::new()),
        );
        padded.push(item);
    }
    padded
}

fn summary_day(item: &IndexMap<String, SummaryValue>) -> Option<NaiveDate> {
    match item.get("startTime") {
        Some(SummaryValue::Day(day)) => Some(*day),
        Some(SummaryValue::Spend(_) | SummaryValue::Breakdown(_)) | None => None,
    }
}
