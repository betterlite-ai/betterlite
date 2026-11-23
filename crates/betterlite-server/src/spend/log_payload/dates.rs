use chrono::format::ParseError;
use chrono::{DateTime, NaiveDateTime, Utc};

pub const DETAIL_DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

// the window a payload callback may narrow its search to; neither bound touches the row lookup
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailDates {
    pub start_time_utc: Option<DateTime<Utc>>,
    pub end_time_utc: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum DetailDateError {
    #[error("{field} {value:?} is not a %Y-%m-%d %H:%M:%S time")]
    Format {
        field: &'static str,
        value: String,
        #[source]
        source: ParseError,
    },
}

// datetime.strptime(value, "%Y-%m-%d %H:%M:%S").replace(tzinfo=timezone.utc), start first, the text as sent
pub fn decode_detail_dates(
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<DetailDates, DetailDateError> {
    Ok(DetailDates {
        start_time_utc: start_date
            .map(|text| utc_time("start_date", text))
            .transpose()?,
        end_time_utc: end_date
            .map(|text| utc_time("end_date", text))
            .transpose()?,
    })
}

fn utc_time(field: &'static str, text: &str) -> Result<DateTime<Utc>, DetailDateError> {
    NaiveDateTime::parse_from_str(text, DETAIL_DATE_FORMAT)
        .map(|naive| naive.and_utc())
        .map_err(|source| DetailDateError::Format {
            field,
            value: text.to_owned(),
            source,
        })
}
