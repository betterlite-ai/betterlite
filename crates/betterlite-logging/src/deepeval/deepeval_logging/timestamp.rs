use chrono::{DateTime, SecondsFormat, TimeZone, Utc};

mod tests;

#[must_use]
pub fn to_zod_compatible_iso<Tz>(moment: &DateTime<Tz>) -> String
where
    Tz: TimeZone,
{
    moment
        .with_timezone(&Utc)
        // DeepEval's zod parser expects exactly three fraction digits, which chrono writes whether or not they are zero
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}
