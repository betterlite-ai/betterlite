use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use rust_decimal::Decimal;
use serde::Serialize;

use super::timestamp::{USAGE_START, parse_timestamp, utc_iso};
use super::transport::Clock;
use crate::finops::cloudzero::records::{CbfRecord, CbfValue};

pub type ApiRecord = BTreeMap<String, String>;

#[derive(Debug, Serialize)]
pub struct BillingDrop<'drop> {
    pub month: String,
    pub operation: &'drop str,
    pub data: Vec<ApiRecord>,
}

pub(super) fn format_decimal(value: Decimal) -> String {
    let text = value.round_dp(10).to_string();
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

pub(super) fn convert_record<Time>(
    row: &CbfRecord,
    user_timezone: &TimeZone,
    clock: &Time,
) -> ApiRecord
where
    Time: Clock,
{
    let mut record = ApiRecord::new();
    for (key, value) in row {
        let text = match value {
            CbfValue::None => continue,
            CbfValue::Text(text) => text.clone(),
            CbfValue::Integer(number) => number.to_string(),
            CbfValue::Money(amount) => format_decimal(*amount),
            CbfValue::DateTime(moment) => utc_iso(*moment),
        };
        record.insert(key.clone(), text);
    }
    if let Some(start) = record.get_mut(USAGE_START) {
        let moment = if start.is_empty() {
            clock.now()
        } else {
            parse_timestamp(start, user_timezone).unwrap_or_else(|_| clock.now())
        };
        *start = utc_iso(moment);
    }
    record
}
