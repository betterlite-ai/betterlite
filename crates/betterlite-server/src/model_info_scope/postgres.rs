use std::convert::Infallible;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::model::{ModelError, ProxyModelBinding};
use betterlite_db::resource_repository::table::{KeyFilter, ResourceActions};
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::ConfigSyncTableActions;
use betterlite_router::model_group_lookups::access_groups::AccessGroupMap;

use super::compose::{ModelCreator, ScopeReads, TeamSelection, UserScope};
use super::error::{ScopeError, UserRowReadError};
use super::teams::TeamScope;
use crate::team_fields::tables::user_table;

// A null $1 asks for every team. LiteLLM's row model declares these lists non-null, so a stored null arrives empty.
const TEAMS_SQL: &str = r#"SELECT (to_jsonb(team) || jsonb_build_object('admins', COALESCE(to_jsonb(team.admins), '[]'::jsonb), 'members', COALESCE(to_jsonb(team.members), '[]'::jsonb), 'models', COALESCE(to_jsonb(team.models), '[]'::jsonb)))::TEXT FROM "LiteLLM_TeamTable" team WHERE $1::text[] IS NULL OR team.team_id = ANY($1::text[])"#;
// LiteLLM reads row.access_model_names or [], so a stored null list arrives as an empty one.
const ACCESS_GROUPS_SQL: &str = r#"SELECT (to_jsonb(grp) || jsonb_build_object('access_model_names', COALESCE(to_jsonb(grp.access_model_names), '[]'::jsonb), 'access_mcp_server_ids', COALESCE(to_jsonb(grp.access_mcp_server_ids), '[]'::jsonb), 'access_agent_ids', COALESCE(to_jsonb(grp.access_agent_ids), '[]'::jsonb), 'assigned_team_ids', COALESCE(to_jsonb(grp.assigned_team_ids), '[]'::jsonb), 'assigned_key_ids', COALESCE(to_jsonb(grp.assigned_key_ids), '[]'::jsonb)))::TEXT FROM "LiteLLM_AccessGroupTable" grp WHERE grp.access_group_id = ANY($1::text[])"#;

#[derive(Clone, Copy)]
pub struct PostgresScopeReads<'db> {
    pub client: PrismaClient<'db>,
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

async fn fetch_creator(
    client: PrismaClient<'_>,
    model_id: &str,
) -> Result<Option<ModelCreator>, ModelError> {
    let repository = PrismaTableRepository::new(
        Some(client),
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
    let model = ProxyModelTable::try_from(row)?;
    Ok(Some(ModelCreator {
        created_by: model.fields.created_by,
    }))
}

// LiteLLM's per-row LiteLLM_TeamTable.model_validate: a stored row the model refuses is an error, never a skipped team.
pub(crate) fn team_scope_from_json(text: &str) -> Result<TeamScope, TeamQueryError> {
    let record = TeamRecord::parse(text.as_bytes())?;
    let decoded = decode_team_record(record.as_ref())?;
    let Some(team) = decoded else {
        return Err(TeamQueryError::from(TeamRecordError::NotAnObject));
    };
    Ok(TeamScope {
        team_id: team.fields.base.team_id,
        models: team.fields.base.models,
        access_group_ids: team.fields.base.access_group_ids.unwrap_or_default(),
    })
}

async fn read_team_rows(
    store: &PostgresStore,
    team_ids: Option<&[String]>,
) -> Result<Vec<TeamScope>, TeamQueryError> {
    let client = store.session().await?;
    let rows = client
        .query(TEAMS_SQL, &[&team_ids])
        .await
        .map_err(backend::<Infallible>)?;
    let mut teams = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row.try_get(0).map_err(backend::<Infallible>)?;
        teams.push(team_scope_from_json(&text)?);
    }
    Ok(teams)
}

async fn read_access_group_rows(
    store: &PostgresStore,
    access_group_ids: &[String],
) -> Result<AccessGroupMap, TableError> {
    let client = store.session().await?;
    let rows = client
        .query(ACCESS_GROUPS_SQL, &[&access_group_ids])
        .await
        .map_err(backend::<Infallible>)?;
    let texts: Vec<String> = rows
        .iter()
        .map(|row| row.try_get(0))
        .collect::<Result<_, _>>()
        .map_err(backend::<Infallible>)?;
    let mut groups = AccessGroupMap::new();
    for text in &texts {
        let group: AccessGroupTable = serde_json::from_str(text).map_err(TableError::Row)?;
        groups.insert(group.access_group_id, group.access_model_names);
    }
    Ok(groups)
}

#[async_trait]
impl ScopeReads for PostgresScopeReads<'_> {
    async fn model_creator(&self, model_id: &str) -> Result<Option<ModelCreator>, ScopeError> {
        fetch_creator(self.client, model_id)
            .await
            .map_err(|source| ScopeError::ModelRead {
                model_id: model_id.to_owned(),
                source: Box::new(source),
            })
    }

    async fn user_scope(&self, user_id: &str) -> Result<Option<UserScope>, ScopeError> {
        let session = read_store(self.client)
            .session()
            .await
            .map_err(|source| ScopeError::UserRead(Box::new(UserRowReadError::from(source))))?;
        let repository = user_table(Some(&session));
        let user = repository
            .find_by_id(user_id)
            .await
            .map_err(|source| ScopeError::UserRead(Box::new(UserRowReadError::from(source))))?;
        Ok(user.map(|row| UserScope {
            teams: row.teams,
            models: row.models,
        }))
    }

    async fn team_scopes(&self, selection: &TeamSelection) -> Result<Vec<TeamScope>, ScopeError> {
        let store = read_store(self.client);
        let teams = match selection {
            TeamSelection::All => read_team_rows(store, None).await,
            TeamSelection::Ids(ids) => read_team_rows(store, Some(ids.as_slice())).await,
        };
        teams.map_err(|source| ScopeError::TeamRead(Box::new(source)))
    }

    async fn access_group_models(
        &self,
        access_group_ids: &[String],
    ) -> Result<AccessGroupMap, ScopeError> {
        read_access_group_rows(read_store(self.client), access_group_ids)
            .await
            .map_err(|source| ScopeError::AccessGroupRead(Box::new(source)))
    }
}
