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

    fn text_value(&self, column: &str) -> Result<Option<&str>, RowError> {
        match self.value(column)? {
            DatabaseValue::Null => Ok(None),
            DatabaseValue::Text(value) => Ok(Some(value.as_str())),
            DatabaseValue::Integer(_) | DatabaseValue::Number(_) => Err(RowError::ColumnType {
                column: column.to_owned(),
                expected: "text",
            }),
        }
    }

    pub fn optional_text(&self, column: &str) -> Result<Option<String>, RowError> {
        Ok(self.text_value(column)?.map(str::to_owned))
    }

    pub fn text(&self, column: &str) -> Result<String, RowError> {
        self.text_value(column)?
            .map(str::to_owned)
            .ok_or_else(|| RowError::ColumnType {
                column: column.to_owned(),
                expected: "a non-null text",
            })
    }

    pub fn optional_integer(&self, column: &str) -> Result<Option<i64>, RowError> {
        match self.value(column)? {
            DatabaseValue::Null => Ok(None),
            DatabaseValue::Integer(value) => Ok(Some(*value)),
            DatabaseValue::Text(_) | DatabaseValue::Number(_) => Err(RowError::ColumnType {
                column: column.to_owned(),
                expected: "an integer",
            }),
        }
    }

    pub fn integer(&self, column: &str) -> Result<i64, RowError> {
        self.optional_integer(column)?
            .ok_or_else(|| RowError::ColumnType {
                column: column.to_owned(),
                expected: "a non-null integer",
            })
    }

    // a column that is missing or null reads as absent, like a mapping lookup without a default
    pub fn integer_if_present(&self, column: &str) -> Result<Option<i64>, RowError> {
        match self.values.get(column) {
            None | Some(DatabaseValue::Null) => Ok(None),
            Some(DatabaseValue::Integer(value)) => Ok(Some(*value)),
            Some(DatabaseValue::Text(_) | DatabaseValue::Number(_)) => Err(RowError::ColumnType {
                column: column.to_owned(),
                expected: "an integer",
            }),
        }
    }

    pub fn optional_decimal(&self, column: &str) -> Result<Option<Decimal>, RowError> {
        match self.value(column)? {
            DatabaseValue::Null => Ok(None),
            DatabaseValue::Number(value) => Ok(Some(*value)),
            DatabaseValue::Text(_) | DatabaseValue::Integer(_) => Err(RowError::ColumnType {
                column: column.to_owned(),
                expected: "a number",
            }),
        }
    }

    pub fn decimal(&self, column: &str) -> Result<Decimal, RowError> {
        self.optional_decimal(column)?
            .ok_or_else(|| RowError::ColumnType {
                column: column.to_owned(),
                expected: "a non-null number",
            })
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
