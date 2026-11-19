use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::bindings::AuditLog;
use betterlite_db::auxiliary_repository::repository::{TableBinding, TableError};
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::Utc;
use secrecy::ExposeSecret as _;

use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::{AuditLogRepository, AuditRepositoryError};

pub struct PostgresAuditLogRepository {
    pub store: PostgresStore,
}

#[async_trait]
impl AuditLogRepository for PostgresAuditLogRepository {
    fn available(&self) -> bool {
        self.store.pool.is_some()
    }

    // Prisma's data={model_dump(exclude_none=True)} inserts only the keys the record carries, so a None keeps the column default
    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        let document = audit_document(record);
        let sql = insert_statement(&document);
        let json = serde_json::to_string(&document)
            .map_err(|source| AuditRepositoryError::Table(TableError::Row(source)))?;
        let client = self
            .store
            .session()
            .await
            .map_err(|source| AuditRepositoryError::Table(TableError::Database(source)))?;
        client
            .execute(&sql, &[&json])
            .await
            .map_err(|source| AuditRepositoryError::Table(TableError::Database(backend(source))))?;
        Ok(())
    }
}

fn audit_document(record: &AuditLogRecord) -> BTreeMap<String, JsonValue> {
    let mut document = BTreeMap::new();
    document.insert("id".to_owned(), JsonValue::String(record.id.clone()));
    document.insert(
        "updated_at".to_owned(),
        JsonValue::String(
            record
                .updated_at
                .with_timezone(&Utc)
                .format("%Y-%m-%dT%H:%M:%S%.3f")
                .to_string(),
        ),
    );
    if let Some(changed_by) = &record.changed_by {
        document.insert(
            "changed_by".to_owned(),
            JsonValue::String(changed_by.clone()),
        );
    }
    if let Some(api_key) = &record.changed_by_api_key {
        document.insert(
            "changed_by_api_key".to_owned(),
            JsonValue::String(api_key.expose_secret().to_owned()),
        );
    }
    document.insert(
        "action".to_owned(),
        JsonValue::String(record.action.as_str().to_owned()),
    );
    document.insert(
        "table_name".to_owned(),
        JsonValue::String(record.table_name.as_str().to_owned()),
    );
    document.insert(
        "object_id".to_owned(),
        JsonValue::String(record.object_id.clone()),
    );
    if let Some(before) = &record.before_value {
        document.insert("before_value".to_owned(), before.clone());
    }
    if let Some(updated) = &record.updated_values {
        document.insert("updated_values".to_owned(), updated.clone());
    }
    document
}

// The shared statement builder covers typed bindings; LiteLLM's audit row is written as a partial document
fn insert_statement(document: &BTreeMap<String, JsonValue>) -> String {
    let table = format!("\"{}\"", <AuditLog as TableBinding>::TABLE);
    let columns: Vec<String> = document.keys().map(|name| format!("\"{name}\"")).collect();
    let values: Vec<String> = columns.iter().map(|column| format!("r.{column}")).collect();
    format!(
        "INSERT INTO {table} AS t ({}) SELECT {} FROM \
         jsonb_populate_record(NULL::{table}, $1::text::jsonb) AS r",
        columns.join(", "),
        values.join(", ")
    )
}
