use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

// B2317 reads the long-form duration syntax in the proxy's default time zone; a model budget shares it
pub struct ZoneDurations<'database> {
    pub database: &'database TimeZoneDatabase,
}

impl DurationParser for ZoneDurations<'_> {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        let parsed = duration_in_seconds(spec, now, None, self.database).ok()?;
        Some(parsed.seconds)
    }
}
