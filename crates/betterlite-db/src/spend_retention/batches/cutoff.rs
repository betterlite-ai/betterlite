use std::error::Error;

use bytes::BytesMut;
use chrono::{DateTime, NaiveDate, Utc};
use tokio_postgres::types::{IsNull, ToSql, Type, to_sql_checked};

const DAY_FORMAT: &str = "%Y-%m-%d";

// rows strictly older than the cutoff are expired: an instant, or an ISO calendar day for tables keyed by day
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cutoff {
    Timestamp(DateTime<Utc>),
    Day(NaiveDate),
}

impl Cutoff {
    #[must_use]
    pub const fn cast(self) -> &'static str {
        match self {
            Self::Timestamp(_) => "timestamptz",
            Self::Day(_) => "text",
        }
    }

    // datetime.isoformat: microseconds only when they are not zero, and UTC spelled +00:00
    #[must_use]
    pub fn text(self) -> String {
        match self {
            Self::Timestamp(moment) if moment.timestamp_subsec_micros() == 0 => {
                moment.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
            }
            Self::Timestamp(moment) => moment.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string(),
            Self::Day(day) => day.format(DAY_FORMAT).to_string(),
        }
    }
}

impl ToSql for Cutoff {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Send + Sync>> {
        match self {
            Self::Timestamp(moment) => moment.to_sql(ty, out),
            Self::Day(day) => day.format(DAY_FORMAT).to_string().to_sql(ty, out),
        }
    }

    fn accepts(ty: &Type) -> bool {
        <DateTime<Utc> as ToSql>::accepts(ty) || <String as ToSql>::accepts(ty)
    }

    to_sql_checked!();
}
