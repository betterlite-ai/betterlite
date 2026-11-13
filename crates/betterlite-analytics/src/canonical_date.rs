use chrono::NaiveDate;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeDenial {
    Forbidden,
    NotFound,
}

impl ScopeDenial {
    #[must_use]
    pub const fn status_code(self) -> u16 {
        match self {
            Self::Forbidden => 403,
            Self::NotFound => 404,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDenied {
    pub status: ScopeDenial,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidDateRange {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PublicFailure {
    #[error("{}", .0.reason)]
    ScopeDenied(ScopeDenied),
    #[error("{}", .0.reason)]
    InvalidDateRange(InvalidDateRange),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicErrorDetail {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicError {
    pub status_code: u16,
    pub detail: PublicErrorDetail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalDateRange {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

#[must_use]
pub fn parse_canonical_date(value: &str) -> Option<NaiveDate> {
    let [
        year_first,
        year_second,
        year_third,
        year_fourth,
        b'-',
        month_first,
        month_second,
        b'-',
        day_first,
        day_second,
    ] = value.as_bytes()
    else {
        return None;
    };
    let digits = [
        year_first,
        year_second,
        year_third,
        year_fourth,
        month_first,
        month_second,
        day_first,
        day_second,
    ];
    if digits.iter().any(|digit| !digit.is_ascii_digit()) {
        return None;
    }
    // Python's date rejects year 0, which chrono would accept
    if [year_first, year_second, year_third, year_fourth] == [&b'0', &b'0', &b'0', &b'0'] {
        return None;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

pub fn parse_canonical_date_range(
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<CanonicalDateRange, InvalidDateRange> {
    let (Some(start_text), Some(end_text)) = (start_date, end_date) else {
        return Err(InvalidDateRange {
            reason: "Please provide start_date and end_date".to_owned(),
        });
    };
    let (Some(start), Some(end)) = (
        parse_canonical_date(start_text),
        parse_canonical_date(end_text),
    ) else {
        return Err(InvalidDateRange {
            reason: "start_date and end_date must be valid YYYY-MM-DD dates".to_owned(),
        });
    };
    Ok(CanonicalDateRange { start, end })
}

#[must_use]
pub fn raise_public(failure: PublicFailure) -> PublicError {
    match failure {
        PublicFailure::ScopeDenied(denied) => PublicError {
            status_code: denied.status.status_code(),
            detail: PublicErrorDetail {
                error: denied.reason,
            },
        },
        PublicFailure::InvalidDateRange(range) => PublicError {
            status_code: 400,
            detail: PublicErrorDetail {
                error: range.reason,
            },
        },
    }
}

mod tests;
