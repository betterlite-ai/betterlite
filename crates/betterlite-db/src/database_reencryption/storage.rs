use std::collections::BTreeSet;
use std::future::Future;

use deadpool_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::MigrationError;
use super::registry::{DISCOVERY_SQL, SecretColumn, registered_columns, rows_sql, update_sql};
use crate::encryption_scan::MigrationJson;
use crate::encryption_scan::decode_migration_json;
use crate::encryption_scan::secret_text::SecretText;

// SQL NULL is None; a JSON null is Some(MigrationJson::Null), so the two never merge
pub struct SecretRow {
    pub primary_key: String,
    pub value: Option<MigrationJson>,
}

pub trait SecretStorage {
    fn secret_columns(
        &self,
    ) -> impl Future<Output = Result<Vec<SecretColumn>, MigrationError>> + Send;

    fn rows(
        &self,
        column: &SecretColumn,
    ) -> impl Future<Output = Result<Vec<SecretRow>, MigrationError>> + Send;

    fn replace(
        &self,
        column: &SecretColumn,
        old: &str,
        new: &str,
        primary_key: &str,
    ) -> impl Future<Output = Result<bool, MigrationError>> + Send;
}

impl SecretStorage for Client {
    async fn secret_columns(&self) -> Result<Vec<SecretColumn>, MigrationError> {
        let records = self
            .query(DISCOVERY_SQL, &[])
            .await
            .map_err(MigrationError::Read)?;
        let mut present = BTreeSet::new();
        for record in records {
            let table: String = record.try_get(0).map_err(MigrationError::Read)?;
            let column: String = record.try_get(1).map_err(MigrationError::Read)?;
            present.insert((table, column));
        }
        Ok(registered_columns()
            .filter(|column| present.contains(&(column.table.to_owned(), column.column.to_owned())))
            .collect())
    }

    async fn rows(&self, column: &SecretColumn) -> Result<Vec<SecretRow>, MigrationError> {
        let sql = rows_sql(column);
        let records = self
            .query(sql.as_str(), &[])
            .await
            .map_err(MigrationError::Read)?;
        records
            .iter()
            .map(|record| decode_row(record, column))
            .collect()
    }

    async fn replace(
        &self,
        column: &SecretColumn,
        old: &str,
        new: &str,
        primary_key: &str,
    ) -> Result<bool, MigrationError> {
        let sql = update_sql(column);
        let parameters: [&(dyn ToSql + Sync); 3] = [&new, &primary_key, &old];
        let updated = self
            .execute(sql.as_str(), &parameters)
            .await
            .map_err(MigrationError::Write)?;
        Ok(updated > 0)
    }
}

fn invalid_row(column: &SecretColumn, source: tokio_postgres::Error) -> MigrationError {
    MigrationError::InvalidRow {
        table: column.table.to_owned(),
        column: column.column.to_owned(),
        source: Some(source),
    }
}

fn decode_row(record: &Row, column: &SecretColumn) -> Result<SecretRow, MigrationError> {
    let key: Option<String> = record
        .try_get(0)
        .map_err(|source| invalid_row(column, source))?;
    let primary_key = key.ok_or_else(|| MigrationError::InvalidRow {
        table: column.table.to_owned(),
        column: column.column.to_owned(),
        source: None,
    })?;
    let value = if column.is_json {
        let raw: Option<String> = record
            .try_get(1)
            .map_err(|source| invalid_row(column, source))?;
        raw.map(|text| decode_migration_json(text.as_bytes()))
            .transpose()
            .map_err(MigrationError::Decode)?
    } else {
        let raw: Option<String> = record
            .try_get(1)
            .map_err(|source| invalid_row(column, source))?;
        raw.map(|text| MigrationJson::String(SecretText(secrecy::SecretString::from(text))))
    };
    Ok(SecretRow { primary_key, value })
}
