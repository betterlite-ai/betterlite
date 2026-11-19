use std::collections::BTreeMap;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::budget_reset::ResetClocks;
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

pub(crate) const DATABASE: fn() -> TimeZoneDatabase = TimeZoneDatabase::from_env;

pub(crate) fn parse(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn settings() -> BudgetResetSettings {
    BudgetResetSettings::default()
}

pub(crate) fn clocks() -> ResetClocks<'static> {
    let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    ResetClocks {
        reset_now: now,
        calendar_now: now,
        calendar_timezone: None,
    }
}

pub(crate) fn object(value: &JsonData) -> &BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = value else {
        panic!("expected an object");
    };
    fields
}
