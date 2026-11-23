use chrono::NaiveDate;

// datetime.strptime's ValueError, which view_spend_logs prefixes and returns as a 500
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("time data '{value}' does not match format '%Y-%m-%d'")]
pub struct LegacyDateError {
    value: String,
}

// a date is a plain day read as UTC midnight: a time suffix fails the way strptime refuses it
pub fn parse_legacy_date(text: &str) -> Result<NaiveDate, LegacyDateError> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_unparsed| LegacyDateError {
        value: text.to_owned(),
    })
}
