use std::cmp::Reverse;

use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use super::rows::{DailyTagSpend, TagSpendBody, TagSpendRow};

const SPEND_PLACES: u32 = 4;

#[derive(Debug, thiserror::Error)]
pub(super) enum TagAggregationError {
    #[error("summing the daily tag spend overflowed Decimal's range")]
    AmountOverflow,
    #[error("summing the daily tag log counts overflowed i64's range")]
    CountOverflow,
}

// ui_get_spend_by_tags: sum every date per tag, rank by spend and round each total to four places
pub(super) fn tag_spend_body(rows: &[DailyTagSpend]) -> Result<TagSpendBody, TagAggregationError> {
    let mut totals: IndexMap<Option<String>, (Decimal, i64)> = IndexMap::new();
    for row in rows {
        let (spend, log_count) = totals
            .entry(row.individual_request_tag.clone())
            .or_insert((Decimal::ZERO, 0));
        *spend = spend
            .checked_add(row.total_spend)
            .ok_or(TagAggregationError::AmountOverflow)?;
        *log_count = log_count
            .checked_add(row.log_count)
            .ok_or(TagAggregationError::CountOverflow)?;
    }
    let mut ranked: Vec<(Option<String>, (Decimal, i64))> = totals.into_iter().collect();
    ranked.sort_by_key(|entry| Reverse(entry.1.0));
    let spend_per_tag = ranked
        .into_iter()
        .map(|(name, (spend, log_count))| TagSpendRow {
            name,
            spend: spend
                .round_dp_with_strategy(SPEND_PLACES, RoundingStrategy::MidpointNearestEven),
            log_count,
        })
        .collect();
    Ok(TagSpendBody { spend_per_tag })
}
