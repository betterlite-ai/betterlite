use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::JsonData;
use crate::budget_reset_policy::retry::RetryMode;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::database_faults::{DbError, ErrorType};
use crate::identity_budget_reset::faults::read_fault;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;

use super::source::WindowSource;

#[derive(Debug, thiserror::Error)]
pub enum WindowJsonError {
    #[error("the budget_limits JSON on row {row} is unreadable: {source}")]
    Unreadable {
        row: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the budget_limits value on row {row} is not a list of windows")]
    NotAList { row: String },
    #[error("row {row} has a budget window that is not an object")]
    NotAWindow { row: String },
    #[error("row {row} has a budget window without a budget_duration")]
    MissingDuration { row: String },
    #[error("row {row} has a budget window whose budget_duration is not a string")]
    DurationNotString { row: String },
    #[error("row {row} has a budget window whose reset_at is not a string")]
    ResetAtNotString { row: String },
    #[error("row {row} has a budget window whose max_budget is not a number")]
    MaxBudgetNotNumber { row: String },
    #[error("the budget_limits on row {row} could not be encoded: {source}")]
    Encode {
        row: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowFields<'window> {
    pub(crate) budget_duration: &'window str,
    pub(crate) max_budget: Option<Decimal>,
    pub(crate) reset_at: Option<&'window str>,
}

// The column holds a list, or the string its own last write left there; a null or an empty string carries no windows and a non-list is corruption.
pub(crate) fn decode_budget_limits(row: &str, raw: &str) -> Result<Vec<JsonData>, WindowJsonError> {
    let value: JsonData = parse(row, raw)?;
    match value {
        JsonData::Null => Ok(Vec::new()),
        JsonData::String(inner) if inner.is_empty() => Ok(Vec::new()),
        JsonData::String(inner) => serialized_windows(row, &inner),
        JsonData::Array(windows) => Ok(windows),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::Object(_) => {
            Err(WindowJsonError::NotAList {
                row: row.to_owned(),
            })
        }
    }
}

fn serialized_windows(row: &str, inner: &str) -> Result<Vec<JsonData>, WindowJsonError> {
    match parse(row, inner)? {
        JsonData::Array(windows) => Ok(windows),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Object(_) => Err(WindowJsonError::NotAList {
            row: row.to_owned(),
        }),
    }
}

fn parse(row: &str, text: &str) -> Result<JsonData, WindowJsonError> {
    serde_json::from_str(text).map_err(|source| WindowJsonError::Unreadable {
        row: row.to_owned(),
        source,
    })
}

// window['budget_duration'] and window.get('max_budget') / window.get('reset_at') as the page reads them; a missing duration fails before the expiry check, exactly where the counter key is built.
pub(crate) fn window_fields<'window>(
    row: &str,
    window: &'window JsonData,
) -> Result<WindowFields<'window>, WindowJsonError> {
    let JsonData::Object(fields) = window else {
        return Err(WindowJsonError::NotAWindow {
            row: row.to_owned(),
        });
    };
    Ok(WindowFields {
        budget_duration: duration_field(row, fields)?,
        max_budget: max_budget_field(row, fields)?,
        reset_at: reset_at_field(row, fields)?,
    })
}

fn duration_field<'window>(
    row: &str,
    fields: &'window BTreeMap<String, JsonData>,
) -> Result<&'window str, WindowJsonError> {
    match fields.get("budget_duration") {
        Some(JsonData::String(text)) => Ok(text.as_str()),
        Some(JsonData::Null) | None => Err(WindowJsonError::MissingDuration {
            row: row.to_owned(),
        }),
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => Err(WindowJsonError::DurationNotString {
            row: row.to_owned(),
        }),
    }
}

// A window with no cap carries nothing; a cap of another type is outside the number the window declares, so it names the field instead of reading as no cap the way isinstance would.
fn max_budget_field(
    row: &str,
    fields: &BTreeMap<String, JsonData>,
) -> Result<Option<Decimal>, WindowJsonError> {
    match fields.get("max_budget") {
        Some(JsonData::Number(number)) => Ok(number.as_f64().and_then(Decimal::from_f64)),
        Some(JsonData::Null) | None => Ok(None),
        Some(
            JsonData::Bool(_) | JsonData::String(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => Err(WindowJsonError::MaxBudgetNotNumber {
            row: row.to_owned(),
        }),
    }
}

fn reset_at_field<'window>(
    row: &str,
    fields: &'window BTreeMap<String, JsonData>,
) -> Result<Option<&'window str>, WindowJsonError> {
    match fields.get("reset_at") {
        None | Some(JsonData::Null) => Ok(None),
        Some(JsonData::String(text)) => Ok(Some(text.as_str())),
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => Err(WindowJsonError::ResetAtNotString {
            row: row.to_owned(),
        }),
    }
}

pub(crate) fn set_window_reset_at(
    row: &str,
    window: &mut JsonData,
    text: String,
) -> Result<(), WindowJsonError> {
    let JsonData::Object(fields) = window else {
        return Err(WindowJsonError::NotAWindow {
            row: row.to_owned(),
        });
    };
    fields.insert("reset_at".to_owned(), JsonData::String(text));
    Ok(())
}

pub(crate) fn encode_budget_limits(
    row: &str,
    windows: &[JsonData],
) -> Result<String, WindowJsonError> {
    serde_json::to_string(windows).map_err(|source| WindowJsonError::Encode {
        row: row.to_owned(),
        source,
    })
}

// source.write: json.dumps(windows) through the ORM update, replayed only on a connect fault because a second write would be a second reset.
pub(crate) async fn persist_budget_limits<Reconnect>(
    store: &PostgresStore,
    source: WindowSource,
    row_id: &str,
    payload: &str,
    reconnect: Option<&mut Reconnect>,
) -> Result<(), DbError>
where
    Reconnect: DbReconnect + Send,
{
    let policy = RetryMode::Write.policy(source.write_reason());
    let attempt = move || update_budget_limits(store, source, row_id, payload);
    call_with_db_reconnect_retry(reconnect, attempt, policy).await
}

async fn update_budget_limits(
    store: &PostgresStore,
    source: WindowSource,
    row_id: &str,
    payload: &str,
) -> Result<(), DbError> {
    let sql = source.update_query();
    let client = store.session().await.map_err(|error| read_fault(&error))?;
    let values: [&(dyn ToSql + Sync); 2] = [&row_id, &payload];
    let updated = client
        .execute(sql.as_str(), &values)
        .await
        .map_err(|error| read_fault(&backend(error)))?;
    if updated == 0 {
        return Err(missing_row(source, row_id));
    }
    Ok(())
}

// Prisma's update raises P2025 when the where clause matches nothing; a row deleted under the walk must not pass as written.
fn missing_row(source: WindowSource, row_id: &str) -> DbError {
    let column = source.id_column();
    DbError {
        error_type: ErrorType::RecordNotFound,
        message: format!("no row with {column} = {row_id} matched the budget_limits update"),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

mod tests;
