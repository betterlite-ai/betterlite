use std::convert::Infallible;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::model::ProxyModelBinding;
use betterlite_db::resource_repository::table::{KeyFilter, ResourceActions};
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::ConfigSyncTableActions;

use super::reads::{ModelRowReadError, TeamScopeReadError, TeamScopeReads, UserReadError};
use crate::model_group_data::errors::GroupDataError;
use crate::model_group_data::ports::GroupDatabase as _;
use crate::model_group_data::store::PostgresGroupData;
use crate::team_fields::tables::{team_table, user_table};

// The stored reads the team query makes; a pod without a database fails each one as B714's routed client would.
#[derive(Clone, Copy)]
pub struct PostgresScopeReads<'db> {
    pub client: Option<PrismaClient<'db>>,
}

fn read_store(client: PrismaClient<'_>) -> &PostgresStore {
    if client
        .availability
        .reader_unavailable
        .load(Ordering::SeqCst)
    {
        return client.writer;
    }
    client.reader
}

async fn fetch_user_teams(
    client: Option<PrismaClient<'_>>,
    user_id: &str,
) -> Result<Option<Vec<String>>, UserReadError> {
    let connected = client.ok_or(DbError::<Infallible>::NoDbConnected)?;
    let session = read_store(connected).session().await?;
    let user = user_table(Some(&session)).find_by_id(user_id).await?;
    Ok(user.map(|row| row.teams))
}

async fn fetch_team_models(
    client: Option<PrismaClient<'_>>,
    team_id: &str,
) -> Result<Option<Vec<String>>, TeamQueryError> {
    let Some(connected) = client else {
        return Err(TeamQueryError::from(DbError::NoDbConnected));
    };
    Ok(team_table(Some(connected))?
        .find_by_id(team_id)
        .await?
        .map(|row| row.fields.base.models))
}

// ModelRepository.table.find_many(where={"model_name": {"in": names}}): B725's rows by name, no team predicate.
async fn fetch_model_ids(
    client: Option<PrismaClient<'_>>,
    names: &[String],
) -> Result<Vec<String>, GroupDataError> {
    let rows = PostgresGroupData { client }
        .deployment_find_many(Some(names))
        .await?;
    Ok(rows.into_iter().map(|row| row.model_id).collect())
}

// ModelRepository.table.find_unique(where={"model_id": model_id}), left as stored so B8382 decrypts it.
async fn fetch_model_row(
    client: Option<PrismaClient<'_>>,
    model_id: &str,
) -> Result<Option<ProxyModelTable>, ModelRowReadError> {
    let connected = client.ok_or(DbError::NoDbConnected)?;
    let repository = PrismaTableRepository::new(
        Some(connected),
        redis_config_change_publisher(None),
        ProxyModelBinding,
    );
    let actions: ConfigSyncTableActions<ResourceActions<'_, ProxyModelBinding>> =
        repository.table(ResourceActions::new)?;
    let rows = actions
        .as_ref()
        .find::<JsonData>(KeyFilter::Equal(model_id))
        .await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(ProxyModelTable::try_from(row)?))
}

#[async_trait]
impl TeamScopeReads for PostgresScopeReads<'_> {
    async fn user_teams(&self, user_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError> {
        fetch_user_teams(self.client, user_id)
            .await
            .map_err(TeamScopeReadError::from)
    }

    async fn team_models(&self, team_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError> {
        fetch_team_models(self.client, team_id)
            .await
            .map_err(TeamScopeReadError::from)
    }

    async fn model_ids_by_names(
        &self,
        names: &[String],
    ) -> Result<Vec<String>, TeamScopeReadError> {
        fetch_model_ids(self.client, names)
            .await
            .map_err(TeamScopeReadError::from)
    }

    async fn model_row(
        &self,
        model_id: &str,
    ) -> Result<Option<ProxyModelTable>, TeamScopeReadError> {
        fetch_model_row(self.client, model_id)
            .await
            .map_err(TeamScopeReadError::from)
    }
}
