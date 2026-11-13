use crate::budget_permission_rows::JsonData;
use crate::daily_activity_contracts::{
    DailyActivityRow, EntityRollupRow, ExportRow, GroupingSetsRow, KeyPage, KeySpendRow,
    RollupMetricsRow,
};
use chrono::NaiveDateTime;
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

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationTokenRow {
    pub token: String,
    pub key_alias: Option<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeletedVerificationTokenRow {
    pub row: VerificationTokenRow,
    pub deleted_at: NaiveDateTime,
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

    fn integer_value(&self, column: &str) -> Result<Option<i64>, RowError> {
        match self.value(column)? {
            DatabaseValue::Null => Ok(None),
            DatabaseValue::Integer(value) => Ok(Some(*value)),
            DatabaseValue::Text(_) | DatabaseValue::Number(_) => Err(RowError::ColumnType {
                column: column.to_owned(),
                expected: "an integer",
            }),
        }
    }

    pub fn optional_integer(&self, column: &str) -> Result<Option<i64>, RowError> {
        self.integer_value(column)
    }

    pub fn integer(&self, column: &str) -> Result<i64, RowError> {
        self.integer_value(column)?
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

    pub fn is_absent(&self, column: &str) -> bool {
        matches!(self.values.get(column), None | Some(DatabaseValue::Null))
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

pub fn verification_token_row(row: &Row) -> Result<VerificationTokenRow, RowError> {
    let metadata_text = row
        .try_get::<_, Option<String>>("metadata")
        .map_err(|source| RowError::Decode { source })?;
    let metadata = match metadata_text {
        None => None,
        Some(text) => Some(json_object(&text, "metadata")?),
    };
    Ok(VerificationTokenRow {
        token: row
            .try_get("token")
            .map_err(|source| RowError::Decode { source })?,
        key_alias: row
            .try_get("key_alias")
            .map_err(|source| RowError::Decode { source })?,
        team_id: row
            .try_get("team_id")
            .map_err(|source| RowError::Decode { source })?,
        user_id: row
            .try_get("user_id")
            .map_err(|source| RowError::Decode { source })?,
        metadata,
    })
}

fn json_object(text: &str, column: &str) -> Result<BTreeMap<String, JsonData>, RowError> {
    let value = serde_json::from_str::<JsonData>(text).map_err(|source| RowError::Json {
        column: column.to_owned(),
        source,
    })?;
    match value {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(RowError::ColumnType {
            column: column.to_owned(),
            expected: "a JSON object",
        }),
    }
}

pub fn deleted_verification_token_row(row: &Row) -> Result<DeletedVerificationTokenRow, RowError> {
    Ok(DeletedVerificationTokenRow {
        row: verification_token_row(row)?,
        deleted_at: row
            .try_get("deleted_at")
            .map_err(|source| RowError::Decode { source })?,
    })
}

pub fn daily_activity_row(row: &Row) -> Result<DailyActivityRow, RowError> {
    Ok(DailyActivityRow {
        id: row
            .try_get("id")
            .map_err(|source| RowError::Decode { source })?,
        date: row
            .try_get("date")
            .map_err(|source| RowError::Decode { source })?,
        api_key: row
            .try_get("api_key")
            .map_err(|source| RowError::Decode { source })?,
        model: row
            .try_get("model")
            .map_err(|source| RowError::Decode { source })?,
        model_group: row
            .try_get("model_group")
            .map_err(|source| RowError::Decode { source })?,
        custom_llm_provider: row
            .try_get("custom_llm_provider")
            .map_err(|source| RowError::Decode { source })?,
        mcp_namespaced_tool_name: row
            .try_get("mcp_namespaced_tool_name")
            .map_err(|source| RowError::Decode { source })?,
        endpoint: row
            .try_get("endpoint")
            .map_err(|source| RowError::Decode { source })?,
        prompt_tokens: row
            .try_get("prompt_tokens")
            .map_err(|source| RowError::Decode { source })?,
        completion_tokens: row
            .try_get("completion_tokens")
            .map_err(|source| RowError::Decode { source })?,
        cache_read_input_tokens: row
            .try_get("cache_read_input_tokens")
            .map_err(|source| RowError::Decode { source })?,
        cache_creation_input_tokens: row
            .try_get("cache_creation_input_tokens")
            .map_err(|source| RowError::Decode { source })?,
        compression_saved_tokens: row
            .try_get("compression_saved_tokens")
            .map_err(|source| RowError::Decode { source })?,
        api_requests: row
            .try_get("api_requests")
            .map_err(|source| RowError::Decode { source })?,
        successful_requests: row
            .try_get("successful_requests")
            .map_err(|source| RowError::Decode { source })?,
        failed_requests: row
            .try_get("failed_requests")
            .map_err(|source| RowError::Decode { source })?,
        total_response_time_ms: row
            .try_get("total_response_time_ms")
            .map_err(|source| RowError::Decode { source })?,
        timed_requests: row
            .try_get("timed_requests")
            .map_err(|source| RowError::Decode { source })?,
        compression_savings_spend: decimal_column(row, "compression_savings_spend")?,
        prompt_caching_savings_spend: decimal_column(row, "prompt_caching_savings_spend")?,
        gateway_cache_savings_spend: decimal_column(row, "gateway_injected_caching_savings_spend")?,
        autorouter_savings_spend: decimal_column(row, "autorouter_savings_spend")?,
        spend: decimal_column(row, "spend")?,
    })
}

fn decimal_column(row: &Row, column: &str) -> Result<Decimal, RowError> {
    let value = row
        .try_get::<_, f64>(column)
        .map_err(|source| RowError::Decode { source })?;
    Decimal::from_f64(value).ok_or_else(|| RowError::NonFiniteNumber {
        column: column.to_owned(),
    })
}

fn metrics_row(row: &DatabaseRow) -> Result<RollupMetricsRow, RowError> {
    Ok(RollupMetricsRow {
        date: row.optional_text("date")?,
        api_key: row.optional_text("api_key")?,
        spend: row.optional_decimal("spend")?,
        ptu_flat_cost: row.optional_decimal("ptu_flat_cost")?,
        prompt_tokens: row.optional_integer("prompt_tokens")?,
        completion_tokens: row.optional_integer("completion_tokens")?,
        cache_read_input_tokens: row.optional_integer("cache_read_input_tokens")?,
        cache_creation_input_tokens: row.optional_integer("cache_creation_input_tokens")?,
        compression_saved_tokens: row.optional_integer("compression_saved_tokens")?,
        compression_savings_spend: row.optional_decimal("compression_savings_spend")?,
        prompt_caching_savings_spend: row.optional_decimal("prompt_caching_savings_spend")?,
        gateway_cache_savings_spend: row
            .optional_decimal("gateway_injected_caching_savings_spend")?,
        autorouter_savings_spend: row.optional_decimal("autorouter_savings_spend")?,
        api_requests: row.optional_integer("api_requests")?,
        successful_requests: row.optional_integer("successful_requests")?,
        failed_requests: row.optional_integer("failed_requests")?,
        total_response_time_ms: row.optional_integer("total_response_time_ms")?,
        timed_requests: row.optional_integer("timed_requests")?,
    })
}

pub fn grouping_rows(rows: &[DatabaseRow]) -> Result<Vec<GroupingSetsRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(GroupingSetsRow {
            metrics: metrics_row(row)?,
            model: row.optional_text("model")?,
            model_group: row.optional_text("model_group")?,
            custom_llm_provider: row.optional_text("custom_llm_provider")?,
            mcp_namespaced_tool_name: row.optional_text("mcp_namespaced_tool_name")?,
            endpoint: row.optional_text("endpoint")?,
            group_level: row.integer("group_level")?,
            distinct_api_keys: row.optional_integer("distinct_api_keys")?,
        });
    }
    Ok(decoded)
}

pub fn entity_rows(rows: &[DatabaseRow]) -> Result<Vec<EntityRollupRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(EntityRollupRow {
            metrics: metrics_row(row)?,
            entity_id: row.optional_text("entity_id")?,
            api_key_rolled: row.integer("api_key_rolled")?,
            distinct_api_keys: row.optional_integer("distinct_api_keys")?,
        });
    }
    Ok(decoded)
}

pub fn key_spend_rows(rows: &[DatabaseRow]) -> Result<Vec<KeySpendRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(key_spend_row(row)?);
    }
    Ok(decoded)
}

fn key_spend_row(row: &DatabaseRow) -> Result<KeySpendRow, RowError> {
    Ok(KeySpendRow {
        api_key: row.text("api_key")?,
        spend: row.decimal("spend")?,
        prompt_tokens: row.integer("prompt_tokens")?,
        completion_tokens: row.integer("completion_tokens")?,
        total_tokens: row.integer("total_tokens")?,
        api_requests: row.integer("api_requests")?,
        successful_requests: row.integer("successful_requests")?,
        failed_requests: row.integer("failed_requests")?,
        cache_read_input_tokens: row.integer("cache_read_input_tokens")?,
        cache_creation_input_tokens: row.integer("cache_creation_input_tokens")?,
    })
}

pub fn key_page(rows: &[DatabaseRow]) -> Result<KeyPage, RowError> {
    let total_api_keys = match rows.first() {
        None => 0,
        Some(row) => row.integer_if_present("total_api_keys")?.unwrap_or(0),
    };
    let retained: Vec<DatabaseRow> = rows
        .iter()
        .filter(|row| !row.is_absent("api_key"))
        .cloned()
        .collect();
    Ok(KeyPage {
        rows: key_spend_rows(&retained)?,
        total_api_keys,
    })
}

pub fn export_rows(rows: &[DatabaseRow]) -> Result<Vec<ExportRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(ExportRow {
            date: row.text("date")?,
            entity_id: row.text("entity_id")?,
            entity_alias: row.optional_text("entity_alias")?,
            api_key: row.optional_text("api_key")?,
            key_alias: row.optional_text("key_alias")?,
            user_id: row.optional_text("user_id")?,
            user_email: row.optional_text("user_email")?,
            model: row.optional_text("model")?,
            spend: row.decimal("spend")?,
            flat_cost: row.decimal("flat_cost")?,
            prompt_tokens: row.integer("prompt_tokens")?,
            completion_tokens: row.integer("completion_tokens")?,
            api_requests: row.integer("api_requests")?,
            successful_requests: row.integer("successful_requests")?,
            failed_requests: row.integer("failed_requests")?,
            cache_read_input_tokens: row.integer("cache_read_input_tokens")?,
            cache_creation_input_tokens: row.integer("cache_creation_input_tokens")?,
        });
    }
    Ok(decoded)
}
