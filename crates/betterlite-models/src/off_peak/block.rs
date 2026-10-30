use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use super::schema::{OffPeakBlock, WindowRuleEntry, WindowsField};
use super::weekday::matches_weekdays;
use super::window::{matches_any_window, window_strings};

pub(crate) fn is_off_peak(
    block: &OffPeakBlock,
    moment: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> bool {
    let flat_windows = window_strings(block.hours_utc.as_ref());
    if !flat_windows.is_empty() && matches_any_window(&flat_windows, moment.time()) {
        return true;
    }
    let Some(WindowsField::List(windows)) = block.windows.as_ref() else {
        return false;
    };
    windows.iter().any(|entry| {
        let WindowRuleEntry::Rule(rule) = entry else {
            return false;
        };
        let rule_windows = window_strings(rule.hours_utc.as_ref());
        !rule_windows.is_empty()
            && matches_weekdays(
                moment,
                rule.weekdays.as_ref(),
                block.weekday_timezone.as_ref(),
                database,
            )
            && matches_any_window(&rule_windows, moment.time())
    })
}
