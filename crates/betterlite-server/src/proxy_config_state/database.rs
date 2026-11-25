use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use async_trait::async_trait;
use betterlite_config::sections::settings::{JsonValue, Section};
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_db::table_ops::ConfigParamInvalidator;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::error::ConfigStateError;
use crate::env_value_codec::for_db::encrypt_env_for_db;

pub mod postgres;

// what one save asks the database to do with a persisted section
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionWrite {
    pub section: Section,
    pub changed: BTreeMap<String, JsonValue>,
    pub removed: BTreeSet<String>,
}

// The write half of LiteLLM's ProxyConfig: config rows, cache invalidation and the env cipher, in memory for tests and PostgreSQL for production
#[async_trait]
pub trait ConfigStateDatabase: Send + Sync {
    fn seal_environment(
        &self,
        values: &BTreeMap<String, JsonValue>,
    ) -> Result<BTreeMap<String, JsonValue>, ConfigStateError>;

    async fn merge_section(
        &self,
        write: &SectionWrite,
    ) -> Result<Option<BTreeMap<String, JsonValue>>, ConfigStateError>;

    async fn insert_config(&self, rows: &[(String, JsonValue)]) -> Result<(), ConfigStateError>;

    async fn merge_environment_variables(
        &self,
        sealed: &BTreeMap<String, JsonValue>,
        deleted: &BTreeSet<String>,
    ) -> Result<(), ConfigStateError>;
}

fn apply_section_edits(
    merged: &mut BTreeMap<String, JsonValue>,
    changed: &BTreeMap<String, JsonValue>,
    removed: &BTreeSet<String>,
) {
    merged.retain(|key, _| !removed.contains(key));
    for (key, value) in changed {
        merged.insert(key.clone(), value.clone());
    }
}

// _upsert_changed_config_section: the stored section minus removed keys plus changed ones, None when the row already holds that
#[must_use]
pub fn merge_section_values(
    existing: &BTreeMap<String, JsonValue>,
    changed: &BTreeMap<String, JsonValue>,
    removed: &BTreeSet<String>,
) -> Option<BTreeMap<String, JsonValue>> {
    let mut merged = existing.clone();
    apply_section_edits(&mut merged, changed, removed);
    (merged != *existing).then_some(merged)
}

// The statements one transaction runs for these edits, so the same merge logic serves the row lock and a test double
pub trait ConfigSectionTransaction: Send + Sync {
    fn lock_section(
        &self,
        section: Section,
    ) -> impl Future<Output = Result<(), ConfigStateError>> + Send;

    fn stored_values(
        &self,
        section: Section,
    ) -> impl Future<Output = Result<Option<BTreeMap<String, JsonValue>>, ConfigStateError>> + Send;

    fn write_values(
        &self,
        section: Section,
        values: &BTreeMap<String, JsonValue>,
    ) -> impl Future<Output = Result<(), ConfigStateError>> + Send;
}

pub async fn merge_stored_section<Tx>(
    transaction: &Tx,
    write: &SectionWrite,
) -> Result<Option<BTreeMap<String, JsonValue>>, ConfigStateError>
where
    Tx: ConfigSectionTransaction + ?Sized,
{
    transaction.lock_section(write.section).await?;
    let existing = transaction
        .stored_values(write.section)
        .await?
        .unwrap_or_default();
    let Some(merged) = merge_section_values(&existing, &write.changed, &write.removed) else {
        return Ok(None);
    };
    transaction.write_values(write.section, &merged).await?;
    Ok(Some(merged))
}

pub async fn merge_stored_environment<Tx>(
    transaction: &Tx,
    sealed: &BTreeMap<String, JsonValue>,
    deleted: &BTreeSet<String>,
) -> Result<(), ConfigStateError>
where
    Tx: ConfigSectionTransaction + ?Sized,
{
    let section = Section::EnvironmentVariables;
    transaction.lock_section(section).await?;
    let existing = transaction
        .stored_values(section)
        .await?
        .unwrap_or_default();
    let merged = merge_section_values(&existing, sealed, deleted);
    let row = merged.unwrap_or(existing);
    transaction.write_values(section, &row).await
}

pub struct DatabaseConfigState<'db> {
    pub store: &'db PostgresStore,
    pub cipher: &'db DbCipher,
    pub invalidator: &'db dyn ConfigParamInvalidator,
}

#[async_trait]
impl ConfigStateDatabase for DatabaseConfigState<'_> {
    fn seal_environment(
        &self,
        values: &BTreeMap<String, JsonValue>,
    ) -> Result<BTreeMap<String, JsonValue>, ConfigStateError> {
        let object: JsonMap = values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let sealed = encrypt_env_for_db(&object, self.cipher, None)?;
        Ok(sealed.into_iter().collect())
    }

    async fn merge_section(
        &self,
        write: &SectionWrite,
    ) -> Result<Option<BTreeMap<String, JsonValue>>, ConfigStateError> {
        let mut client = self
            .store
            .session()
            .await
            .map_err(|source| ConfigStateError::Database { source })?;
        let transaction =
            client
                .transaction()
                .await
                .map_err(|source| ConfigStateError::Database {
                    source: backend(source),
                })?;
        let written = merge_stored_section(&transaction, write).await?;
        transaction
            .commit()
            .await
            .map_err(|source| ConfigStateError::Database {
                source: backend(source),
            })?;
        if written.is_some() {
            self.invalidator
                .invalidate_param(write.section.name())
                .await;
        }
        Ok(written)
    }

    async fn insert_config(&self, rows: &[(String, JsonValue)]) -> Result<(), ConfigStateError> {
        if rows.is_empty() {
            return Ok(());
        }
        let client = self
            .store
            .session()
            .await
            .map_err(|source| ConfigStateError::Database { source })?;
        for (name, value) in rows {
            let encoded = postgres::encode_config_value(value)?;
            client
                .execute(postgres::UPSERT_CONFIG_SQL, &[name, &encoded])
                .await
                .map_err(|source| ConfigStateError::Database {
                    source: backend(source),
                })?;
        }
        for (name, _) in rows {
            self.invalidator.invalidate_param(name).await;
        }
        Ok(())
    }

    async fn merge_environment_variables(
        &self,
        sealed: &BTreeMap<String, JsonValue>,
        deleted: &BTreeSet<String>,
    ) -> Result<(), ConfigStateError> {
        let mut client = self
            .store
            .session()
            .await
            .map_err(|source| ConfigStateError::Database { source })?;
        let transaction =
            client
                .transaction()
                .await
                .map_err(|source| ConfigStateError::Database {
                    source: backend(source),
                })?;
        merge_stored_environment(&transaction, sealed, deleted).await?;
        transaction
            .commit()
            .await
            .map_err(|source| ConfigStateError::Database {
                source: backend(source),
            })?;
        self.invalidator
            .invalidate_param(Section::EnvironmentVariables.name())
            .await;
        Ok(())
    }
}
