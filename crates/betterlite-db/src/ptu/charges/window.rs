use std::cmp::Reverse;

use chrono::{DateTime, Days, NaiveDate, Utc};

use super::pricing::PtuModel;

pub const MAX_BACKFILL_DAYS: u64 = 90;

#[must_use]
pub fn lapsed_models(models: &[PtuModel], now: DateTime<Utc>) -> Vec<String> {
    let mut lapsed: Vec<&PtuModel> = models
        .iter()
        .filter(|model| model.effective_to.is_some_and(|end| end <= now))
        .collect();
    lapsed.sort_by_key(|model| Reverse(model.effective_to));
    lapsed
        .iter()
        .map(|model| {
            model
                .model_name
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        })
        .collect()
}

#[must_use]
pub fn backfill_window(models: &[PtuModel], end: NaiveDate) -> Vec<NaiveDate> {
    let earliest = models
        .iter()
        .map(|model| model.effective_from.date_naive())
        .min();
    let Some(earliest) = earliest else {
        return Vec::new();
    };
    let floor = end.checked_sub_days(Days::new(MAX_BACKFILL_DAYS));
    let start = floor.map_or(earliest, |floor| earliest.max(floor));
    let mut days = Vec::new();
    let mut day = start;
    while day <= end {
        days.push(day);
        let Some(next) = day.checked_add_days(Days::new(1)) else {
            break;
        };
        day = next;
    }
    days
}
