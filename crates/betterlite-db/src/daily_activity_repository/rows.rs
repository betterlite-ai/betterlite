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

    fn optional<T>(
        &self,
        column: &str,
        convert: fn(&DatabaseValue) -> Option<T>,
        expected: &'static str,
    ) -> Result<Option<T>, RowError> {
        match self.value(column)? {
            DatabaseValue::Null => Ok(None),
            value => convert(value)
                .map(Some)
                .ok_or_else(|| column_error(column, expected)),
        }
    }

    pub fn optional_text(&self, column: &str) -> Result<Option<String>, RowError> {
        self.optional(column, text_of, "text")
    }

    pub fn text(&self, column: &str) -> Result<String, RowError> {
        self.optional_text(column)?
            .ok_or_else(|| column_error(column, "a non-null text"))
    }

    pub fn optional_integer(&self, column: &str) -> Result<Option<i64>, RowError> {
        self.optional(column, integer_of, "an integer")
    }

    pub fn integer(&self, column: &str) -> Result<i64, RowError> {
        self.optional_integer(column)?
            .ok_or_else(|| column_error(column, "a non-null integer"))
    }

    // a column that is missing or null reads as absent, like a mapping lookup without a default
    pub fn integer_if_present(&self, column: &str) -> Result<Option<i64>, RowError> {
        if self.values.contains_key(column) {
            return self.optional_integer(column);
        }
        Ok(None)
    }

    pub fn optional_decimal(&self, column: &str) -> Result<Option<Decimal>, RowError> {
        self.optional(column, decimal_of, "a number")
    }

    pub fn decimal(&self, column: &str) -> Result<Decimal, RowError> {
        self.optional_decimal(column)?
            .ok_or_else(|| column_error(column, "a non-null number"))
    }
}

fn column_error(column: &str, expected: &'static str) -> RowError {
    RowError::ColumnType {
        column: column.to_owned(),
        expected,
    }
}

fn text_of(value: &DatabaseValue) -> Option<String> {
    match value {
        DatabaseValue::Text(text) => Some(text.clone()),
        _ => None,
    }
}

fn integer_of(value: &DatabaseValue) -> Option<i64> {
    match value {
        DatabaseValue::Integer(number) => Some(*number),
        _ => None,
    }
}

fn decimal_of(value: &DatabaseValue) -> Option<Decimal> {
    match value {
        DatabaseValue::Number(number) => Some(*number),
        _ => None,
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
            .map_err(decode_error)?;
        return Ok(value.map_or(DatabaseValue::Null, DatabaseValue::Text));
    }
    if column_type == &Type::INT8 {
        let value = row.try_get::<_, Option<i64>>(index).map_err(decode_error)?;
        return Ok(value.map_or(DatabaseValue::Null, DatabaseValue::Integer));
    }
    if column_type == &Type::INT4 {
        let value = row.try_get::<_, Option<i32>>(index).map_err(decode_error)?;
        return Ok(value.map_or(DatabaseValue::Null, |value| {
            DatabaseValue::Integer(i64::from(value))
        }));
    }
    if column_type == &Type::INT2 {
        let value = row.try_get::<_, Option<i16>>(index).map_err(decode_error)?;
        return Ok(value.map_or(DatabaseValue::Null, |value| {
            DatabaseValue::Integer(i64::from(value))
        }));
    }
    if column_type == &Type::FLOAT8 {
        let value = row.try_get::<_, Option<f64>>(index).map_err(decode_error)?;
        return number_value(value, name);
    }
    if column_type == &Type::FLOAT4 {
        let value = row.try_get::<_, Option<f32>>(index).map_err(decode_error)?;
        return number_value(value.map(f64::from), name);
    }
    Err(RowError::UnsupportedColumn {
        column: name.to_owned(),
    })
}

fn decode_error(source: tokio_postgres::Error) -> RowError {
    RowError::Decode { source }
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
