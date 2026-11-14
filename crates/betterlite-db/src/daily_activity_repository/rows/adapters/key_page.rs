use super::super::{DatabaseRow, DatabaseValue, RowError};
use super::key_spend::key_spend_rows;
use crate::daily_activity_contracts::KeyPage;

pub fn key_page(rows: &[DatabaseRow]) -> Result<KeyPage, RowError> {
    let total_api_keys = match rows.first() {
        None => 0,
        Some(row) => row.integer_if_present("total_api_keys")?.unwrap_or(0),
    };
    let retained: Vec<DatabaseRow> = rows
        .iter()
        .filter(|row| !matches!(row.values.get("api_key"), None | Some(DatabaseValue::Null)))
        .cloned()
        .collect();
    Ok(KeyPage {
        rows: key_spend_rows(&retained)?,
        total_api_keys,
    })
}
