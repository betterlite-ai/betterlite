use betterlite_core::agents::json::AgentJson;
use betterlite_db::postgres_store::diagnostics::DriverError;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use tokio_postgres::Row;

use super::{ColumnFailure, RowDecodeError};

pub(super) fn required_text(row: &Row, name: &str) -> Result<String, RowDecodeError> {
    row.try_get(name).map_err(|driver| RowDecodeError::Text {
        source: ColumnFailure::Driver(DriverError { driver }),
    })
}

pub(super) fn optional_text(row: &Row, name: &str) -> Result<Option<String>, RowDecodeError> {
    row.try_get(name).map_err(|driver| RowDecodeError::Text {
        source: ColumnFailure::Driver(DriverError { driver }),
    })
}

pub(super) fn optional_text_array(
    row: &Row,
    name: &str,
) -> Result<Option<Vec<String>>, RowDecodeError> {
    row.try_get(name)
        .map_err(|driver| RowDecodeError::TextArray {
            source: ColumnFailure::Driver(DriverError { driver }),
        })
}

pub(super) fn optional_integer(row: &Row, name: &str) -> Result<Option<i32>, RowDecodeError> {
    row.try_get(name).map_err(|driver| RowDecodeError::Integer {
        source: ColumnFailure::Driver(DriverError { driver }),
    })
}

pub(super) fn required_boolean(row: &Row, name: &str) -> Result<bool, RowDecodeError> {
    row.try_get(name).map_err(|driver| RowDecodeError::Boolean {
        source: ColumnFailure::Driver(DriverError { driver }),
    })
}

pub(super) fn required_json(row: &Row, name: &str) -> Result<AgentJson, RowDecodeError> {
    row.try_get(name).map_err(|driver| RowDecodeError::Json {
        source: ColumnFailure::Driver(DriverError { driver }),
    })
}

pub(super) fn optional_json(row: &Row, name: &str) -> Result<Option<AgentJson>, RowDecodeError> {
    row.try_get(name).map_err(|driver| RowDecodeError::Json {
        source: ColumnFailure::Driver(DriverError { driver }),
    })
}

pub(super) fn required_timestamp(row: &Row, name: &str) -> Result<NaiveDateTime, RowDecodeError> {
    row.try_get(name)
        .map_err(|driver| RowDecodeError::Timestamp {
            source: ColumnFailure::Driver(DriverError { driver }),
        })
}

pub(super) fn spend_value(row: &Row) -> Result<Decimal, RowDecodeError> {
    let stored: f64 = row
        .try_get("spend")
        .map_err(|driver| RowDecodeError::Spend {
            source: ColumnFailure::Driver(DriverError { driver }),
        })?;
    Decimal::from_f64(stored).ok_or(RowDecodeError::Spend {
        source: ColumnFailure::Shape,
    })
}
