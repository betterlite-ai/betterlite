use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::bindings::GuardrailsTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_repository::table::{
    KeyFilter, ResourceActions, ResourceBinding, RowWrite,
};
use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::{ConfigSyncTableActions, TableActions};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;

const KEY: &str = "guardrail_id";
const UPDATED_AT: &str = "updated_at";
const STATUS: &str = "status";
const REVIEWED_AT: &str = "reviewed_at";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReviewRow {
    pub guardrail_id: String,
    pub guardrail_name: String,
    pub litellm_params: Option<JsonData>,
    pub guardrail_info: Option<JsonData>,
    pub team_id: Option<String>,
    pub status: String,
    pub submitted_at: Option<Timestamp>,
    pub reviewed_at: Option<Timestamp>,
}

#[derive(Debug, Error)]
pub enum ReviewRowsError {
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("Guardrail with ID {guardrail_id} not found after the update")]
    MissingRow { guardrail_id: String },
}

pub trait ReviewRows: Send + Sync {
    fn find_unique(
        &self,
        guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<ReviewRow>, ReviewRowsError>> + Send;

    fn set_status(
        &self,
        guardrail_id: &str,
        status: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), ReviewRowsError>> + Send;
}

pub struct ReviewRowsBinding;

impl TableBinding for ReviewRowsBinding {
    const NAME: &'static str = GuardrailsTable::NAME;
    const TABLE: &'static str = GuardrailsTable::TABLE;
}

impl ResourceBinding for ReviewRowsBinding {
    const KEY: &'static str = KEY;
    const ID: Option<&'static str> = None;
    const UPDATED_AT: Option<&'static str> = Some(UPDATED_AT);
    type Create = RowWrite;
    type Update = RowWrite;
}

pub struct PostgresReviewRows<'db> {
    client: Option<PrismaClient<'db>>,
    publish: PublishChange<TableError>,
}

impl<'db> PostgresReviewRows<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self { client, publish }
    }

    // The write handle keeps reads after a write and makes the config-sync wrapper publish
    fn table(
        &self,
    ) -> Result<ConfigSyncTableActions<ResourceActions<'db, ReviewRowsBinding>>, ReviewRowsError>
    {
        let repository =
            PrismaTableRepository::new(self.client, Arc::clone(&self.publish), ReviewRowsBinding);
        let actions = repository
            .with_writer()
            .table(ResourceActions::<ReviewRowsBinding>::new)?;
        Ok(actions)
    }
}

impl ReviewRows for PostgresReviewRows<'_> {
    async fn find_unique(&self, guardrail_id: &str) -> Result<Option<ReviewRow>, ReviewRowsError> {
        let actions = self.table()?;
        let rows = actions
            .as_ref()
            .find::<ReviewRow>(KeyFilter::Equal(guardrail_id))
            .await?;
        Ok(rows.into_iter().next())
    }

    async fn set_status(
        &self,
        guardrail_id: &str,
        status: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ReviewRowsError> {
        let stamp = now.to_rfc3339();
        let mut record = BTreeMap::new();
        record.insert(STATUS.to_owned(), JsonData::String(status.to_owned()));
        record.insert(REVIEWED_AT.to_owned(), JsonData::String(stamp.clone()));
        record.insert(UPDATED_AT.to_owned(), JsonData::String(stamp));
        let actions = self.table()?;
        let updated = actions
            .update(RowWrite(record), guardrail_id.to_owned())
            .await?;
        match updated {
            Some(_row) => Ok(()),
            None => Err(ReviewRowsError::MissingRow {
                guardrail_id: guardrail_id.to_owned(),
            }),
        }
    }
}
