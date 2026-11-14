pub mod adapters;
pub mod records;

use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use std::collections::BTreeMap;
use tokio_postgres::Row;
use tokio_postgres::types::Type;

#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseRow {
    pub(crate) values: BTreeMap<String, DatabaseValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseValue {
    Null,
    Text(String),
    Integer(i64),
    Number(Decimal),
}

#[derive(Debug, thiserror::Error)]
pub enum RowError {
    #[error("column {column} is missing from the row")]
    MissingColumn { column: String },
    #[error("column {column} is not {expected}")]
    ColumnType {
        column: String,
        expected: &'static str,
    },
    #[error("column {column} holds a number that is not finite")]
    NonFiniteNumber { column: String },
    #[error("column {column} has a type this database reader does not decode")]
    UnsupportedColumn { column: String },
    #[error("the database row does not match its record")]
    Decode {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("column {column} does not hold valid JSON")]
    Json {
        column: String,
        #[source]
        source: serde_json::Error,
    },
}

impl DatabaseRow {
    pub fn decode(row: &Row) -> Result<Self, RowError> {
        let mut values = BTreeMap::new();
        for (index, column) in row.columns().iter().enumerate() {
            let value = decode_column(row, index, column.name(), column.type_())?;
            values.insert(column.name().to_owned(), value);
        }
        Ok(Self { values })
    }

    fn value(&self, column: &str) -> Result<&DatabaseValue, RowError> {
        self.values
            .get(column)
            .ok_or_else(|| RowError::MissingColumn {
                column: column.to_owned(),
            })
    }

    pub fn optional<T: ColumnValue>(&self, column: &str) -> Result<Option<T>, RowError> {
        match self.value(column)? {
            DatabaseValue::Null => Ok(None),
            value => T::from_value(value)
                .map(Some)
                .ok_or_else(|| column_error(column, T::EXPECTED)),
        }
    }

    // a column that is missing or null reads as absent, like a mapping lookup without a default
    pub fn integer_if_present(&self, column: &str) -> Result<Option<i64>, RowError> {
        if self.values.contains_key(column) {
            return self.optional::<i64>(column);
        }
        Ok(None)
    }
}

fn column_error(column: &str, expected: &'static str) -> RowError {
    RowError::ColumnType {
        column: column.to_owned(),
        expected,
    }
}

pub trait ColumnValue: Sized {
    const EXPECTED: &'static str;

    fn from_value(value: &DatabaseValue) -> Option<Self>;
}

impl ColumnValue for String {
    const EXPECTED: &'static str = "text";

    fn from_value(value: &DatabaseValue) -> Option<Self> {
        match value {
            DatabaseValue::Text(text) => Some(text.clone()),
            _ => None,
        }
    }
}

impl ColumnValue for i64 {
    const EXPECTED: &'static str = "an integer";

    fn from_value(value: &DatabaseValue) -> Option<Self> {
        match value {
            DatabaseValue::Integer(number) => Some(*number),
            _ => None,
        }
    }
}

impl ColumnValue for Decimal {
    const EXPECTED: &'static str = "a number";

    fn from_value(value: &DatabaseValue) -> Option<Self> {
        match value {
            DatabaseValue::Number(number) => Some(*number),
            _ => None,
        }
    }
}

fn decode_column(
    row: &Row,
    index: usize,
    name: &str,
    column_type: &Type,
) -> Result<DatabaseValue, RowError> {
    if column_type == &Type::TEXT
        || column_type == &Type::VARCHAR
        || column_type == &Type::BPCHAR
        || column_type == &Type::NAME
    {
        let value = row
            .try_get::<_, Option<String>>(index)
            .map_err(|source| RowError::Decode { source })?;
        return Ok(value.map_or(DatabaseValue::Null, DatabaseValue::Text));
    }
    if column_type == &Type::INT8 {
        let value = row
            .try_get::<_, Option<i64>>(index)
            .map_err(|source| RowError::Decode { source })?;
        return Ok(value.map_or(DatabaseValue::Null, DatabaseValue::Integer));
    }
    if column_type == &Type::INT4 {
        let value = row
            .try_get::<_, Option<i32>>(index)
            .map_err(|source| RowError::Decode { source })?;
        return Ok(value.map_or(DatabaseValue::Null, |value| {
            DatabaseValue::Integer(i64::from(value))
        }));
    }
    if column_type == &Type::INT2 {
        let value = row
            .try_get::<_, Option<i16>>(index)
            .map_err(|source| RowError::Decode { source })?;
        return Ok(value.map_or(DatabaseValue::Null, |value| {
            DatabaseValue::Integer(i64::from(value))
        }));
    }
    if column_type == &Type::FLOAT8 {
        let value = row
            .try_get::<_, Option<f64>>(index)
            .map_err(|source| RowError::Decode { source })?;
        return number_value(value, name);
    }
    if column_type == &Type::FLOAT4 {
        let value = row
            .try_get::<_, Option<f32>>(index)
            .map_err(|source| RowError::Decode { source })?;
        return number_value(value.map(f64::from), name);
    }
    Err(RowError::UnsupportedColumn {
        column: name.to_owned(),
    })
}

fn number_value(value: Option<f64>, column: &str) -> Result<DatabaseValue, RowError> {
    match value {
        None => Ok(DatabaseValue::Null),
        Some(value) => Decimal::from_f64(value)
            .map(DatabaseValue::Number)
            .ok_or_else(|| RowError::NonFiniteNumber {
                column: column.to_owned(),
            }),
    }
}
