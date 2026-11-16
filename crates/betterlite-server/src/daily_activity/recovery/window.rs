use betterlite_db::daily_activity_contracts::SpendLogsWindow;
use betterlite_models::savings_evidence::billed_at::decode_billed_at;
use chrono::{DateTime, Days, Utc};
use std::collections::BTreeSet;
use std::time::SystemTime;

#[must_use]
pub fn spend_logs_window(dates: &BTreeSet<String>) -> Option<SpendLogsWindow> {
    let parsed: Vec<DateTime<Utc>> = dates
        .iter()
        .filter_map(|raw| decode_billed_at(raw.as_str()))
        .collect();
    let first = *parsed.iter().min()?;
    let last = *parsed.iter().max()?;
    let start = first.checked_sub_days(Days::new(1))?;
    let end = last.checked_add_days(Days::new(2))?;
    Some((SystemTime::from(start), SystemTime::from(end)))
}
