use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use chrono::DateTime;
use chrono::Days;
use chrono::TimeZone as _;
use chrono::Utc;
use speedate::Date as SpeedateDate;
use speedate::DateTime as SpeedateDateTime;
use speedate::DateTimeConfig;
use speedate::TimeConfig;

use super::error::ModelMetricsError;

const MODEL_GROUP_PARAM: &str = "_selected_model_group";
const START_TIME_PARAM: &str = "startTime";
const END_TIME_PARAM: &str = "endTime";
const API_KEY_PARAM: &str = "api_key";
const CUSTOMER_PARAM: &str = "customer";
const UNDEFINED: &str = "undefined";
const NO_FILTER: &str = "null";

#[derive(Default)]
pub(super) struct MetricsQuery {
    pub(super) model_group: Option<String>,
    pub(super) start_time: Option<String>,
    pub(super) end_time: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) customer: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Window {
    pub(super) start: DateTime<Utc>,
    pub(super) end: DateTime<Utc>,
}

impl MetricsQuery {
    pub(super) fn decode(raw: Option<&str>) -> Self {
        let mut query = Self::default();
        let Some(encoded) = raw else {
            return query;
        };
        for (name, value) in form_urlencoded::parse(encoded.as_bytes()) {
            match name.as_ref() {
                MODEL_GROUP_PARAM => query.model_group = Some(value.into_owned()),
                START_TIME_PARAM => query.start_time = Some(value.into_owned()),
                END_TIME_PARAM => query.end_time = Some(value.into_owned()),
                API_KEY_PARAM => query.api_key = Some(value.into_owned()),
                CUSTOMER_PARAM => query.customer = Some(value.into_owned()),
                _ => {}
            }
        }
        query
    }

    pub(super) fn window(
        &self,
        clock: &dyn GatewayRequestClock,
        lookback_days: u64,
    ) -> Result<Window, ModelMetricsError> {
        let now = clock.utc_now();
        let end = match &self.end_time {
            Some(text) => parse_datetime(END_TIME_PARAM, text)?,
            None => now,
        };
        let start = match &self.start_time {
            Some(text) => parse_datetime(START_TIME_PARAM, text)?,
            None => now
                .checked_sub_days(Days::new(lookback_days))
                .unwrap_or(now),
        };
        Ok(Window { start, end })
    }

    pub(super) fn model_group_or(&self, default: &str) -> String {
        self.model_group
            .as_ref()
            .map_or_else(|| default.to_owned(), String::clone)
    }
}

pub(super) fn normalized(value: Option<&str>) -> String {
    let text = value.unwrap_or(NO_FILTER);
    if text == UNDEFINED {
        return NO_FILTER.to_owned();
    }
    text.to_owned()
}

fn to_utc(parsed: &SpeedateDateTime) -> Option<DateTime<Utc>> {
    let nanoseconds = parsed.time.microsecond.checked_mul(1_000)?;
    Utc.timestamp_opt(parsed.timestamp_tz(), nanoseconds)
        .single()
}

fn read_datetime(text: &str) -> Option<DateTime<Utc>> {
    // pydantic cuts a fraction longer than a microsecond, so the parser does too.
    let time = TimeConfig::builder()
        .microseconds_precision_overflow_behavior("truncate".parse().ok()?)
        .build();
    let config = DateTimeConfig::builder().time_config(time).build();
    let parsed = SpeedateDateTime::parse_str_with_config(text, &config).or_else(|_error| {
        let date = SpeedateDate::parse_str(text)?;
        SpeedateDateTime::parse_str(&format!("{date}T00:00:00"))
    });
    to_utc(&parsed.ok()?)
}

fn parse_datetime(parameter: &'static str, text: &str) -> Result<DateTime<Utc>, ModelMetricsError> {
    read_datetime(text).ok_or_else(|| ModelMetricsError::InvalidDatetime {
        parameter,
        value: text.to_owned(),
    })
}
