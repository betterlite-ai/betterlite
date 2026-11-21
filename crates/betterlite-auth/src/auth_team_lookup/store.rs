use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::AccessGroupTable;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamRepository};
use betterlite_db::team_rows::decoder::parse_data;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use tokio_util::task::TaskTracker;

use super::ports::{StoreAnswer, TeamStore, TeamStoreError};
use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::table::{
    AuthReadError, AuthReadTable, AuthTable, Create, FindMany, FindOne, Presence, ReadContext,
    Update,
};

// _TEAM_GRANT_RELATIONS includes litellm_model_table; Prisma names its aliases column model_aliases and reads a null list as []
const TEAM_BY_ID_SQL: &str = r#"SELECT (to_jsonb(t) || jsonb_build_object('admins', COALESCE(to_jsonb(t.admins), '[]'::jsonb), 'members', COALESCE(to_jsonb(t.members), '[]'::jsonb), 'models', COALESCE(to_jsonb(t.models), '[]'::jsonb), 'team_member_permissions', COALESCE(to_jsonb(t.team_member_permissions), '[]'::jsonb), 'access_group_ids', COALESCE(to_jsonb(t.access_group_ids), '[]'::jsonb), 'policies', COALESCE(to_jsonb(t.policies), '[]'::jsonb), 'default_team_member_models', COALESCE(to_jsonb(t.default_team_member_models), '[]'::jsonb), 'litellm_model_table', (SELECT (to_jsonb(m) - 'aliases') || jsonb_build_object('model_aliases', m.aliases) FROM "LiteLLM_ModelTable" m WHERE m.id = t.model_id)))::text FROM "LiteLLM_TeamTable" t WHERE t.team_id = $1"#;
const TEAM_BY_ALIAS_SQL: &str = r#"SELECT (to_jsonb(t) || jsonb_build_object('admins', COALESCE(to_jsonb(t.admins), '[]'::jsonb), 'members', COALESCE(to_jsonb(t.members), '[]'::jsonb), 'models', COALESCE(to_jsonb(t.models), '[]'::jsonb), 'team_member_permissions', COALESCE(to_jsonb(t.team_member_permissions), '[]'::jsonb), 'access_group_ids', COALESCE(to_jsonb(t.access_group_ids), '[]'::jsonb), 'policies', COALESCE(to_jsonb(t.policies), '[]'::jsonb), 'default_team_member_models', COALESCE(to_jsonb(t.default_team_member_models), '[]'::jsonb), 'litellm_model_table', (SELECT (to_jsonb(m) - 'aliases') || jsonb_build_object('model_aliases', m.aliases) FROM "LiteLLM_ModelTable" m WHERE m.id = t.model_id)))::text FROM "LiteLLM_TeamTable" t WHERE t.team_alias = $1"#;
const ACCESS_GROUP_SQL: &str = r#"SELECT (to_jsonb(g) || jsonb_build_object('access_model_names', COALESCE(to_jsonb(g.access_model_names), '[]'::jsonb), 'access_mcp_server_ids', COALESCE(to_jsonb(g.access_mcp_server_ids), '[]'::jsonb), 'access_agent_ids', COALESCE(to_jsonb(g.access_agent_ids), '[]'::jsonb), 'assigned_team_ids', COALESCE(to_jsonb(g.assigned_team_ids), '[]'::jsonb), 'assigned_key_ids', COALESCE(to_jsonb(g.assigned_key_ids), '[]'::jsonb)))::text FROM "LiteLLM_AccessGroupTable" g WHERE g.access_group_id = $1"#;

// the lookup reads by key and by alias only, so the table's other actions are never wired
const UNREAD: AuthReadError<Infallible> = AuthReadError::Repository(DbError::CreateNull);

pub enum RowKey {
    Team(String),
    AccessGroup(String),
}

// one Postgres handle answering the reads the deadline adapter bounds; rows stay JSON text until decoded
pub struct LookupTable {
    pub store: PostgresStore,
}

impl LookupTable {
    async fn rows(&self, sql: &str, value: &str) -> Result<Vec<String>, AuthReadError<Infallible>> {
        let client = self
            .store
            .session()
            .await
            .map_err(AuthReadError::Repository)?;
        let found = client
            .query(sql, &[&value])
            .await
            .map_err(|source| AuthReadError::Repository(backend(source)))?;
        let mut texts = Vec::with_capacity(found.len());
        for row in &found {
            let text: String = row
                .try_get(0_usize)
                .map_err(|source| AuthReadError::Repository(backend(source)))?;
            texts.push(text);
        }
        Ok(texts)
    }
}

#[async_trait]
impl AuthTable for LookupTable {
    type Row = String;
    type Key = RowKey;
    type Filter = String;
    type Include = ();
    type Data = ();
    type Failure = Infallible;

    async fn find_unique(
        &self,
        request: FindOne<RowKey, ()>,
    ) -> Result<Option<String>, AuthReadError<Infallible>> {
        let (sql, value) = match &request.filter {
            RowKey::Team(team_id) => (TEAM_BY_ID_SQL, team_id),
            RowKey::AccessGroup(access_group_id) => (ACCESS_GROUP_SQL, access_group_id),
        };
        Ok(self.rows(sql, value).await?.into_iter().next())
    }

    async fn find_first(
        &self,
        _request: FindOne<String, ()>,
    ) -> Result<Option<String>, AuthReadError<Infallible>> {
        Err(UNREAD)
    }

    async fn find_many(
        &self,
        request: FindMany<String, ()>,
    ) -> Result<Vec<String>, AuthReadError<Infallible>> {
        let Some(team_alias) = request.filter.value() else {
            return Err(UNREAD);
        };
        self.rows(TEAM_BY_ALIAS_SQL, team_alias).await
    }

    async fn update(
        &self,
        _request: Update<RowKey, ()>,
    ) -> Result<Option<String>, AuthReadError<Infallible>> {
        Err(UNREAD)
    }

    async fn create(&self, _request: Create<(), ()>) -> Result<String, AuthReadError<Infallible>> {
        Err(UNREAD)
    }
}

// TeamRepository picks client.db or writer_db; AccessGroupRepository also follows B714's reader outage
pub struct DatabaseTeamStore {
    pub reader: PostgresStore,
    pub writer: PostgresStore,
    pub availability: Arc<Availability>,
    pub default_deadline_seconds: f64,
    pub tracker: Arc<DbLookupStallTracker<SystemClock>>,
    pub tasks: TaskTracker,
}

impl DatabaseTeamStore {
    fn open(
        &self,
        store: &PostgresStore,
        adapter: AuthReadAdapter,
        label: &str,
    ) -> AuthReadTable<LookupTable> {
        let context = ReadContext {
            default_deadline_seconds: self.default_deadline_seconds,
            tracker: Arc::clone(&self.tracker),
            tasks: self.tasks.clone(),
        };
        let table = LookupTable {
            store: store.clone(),
        };
        adapter.open(Arc::new(table), label.to_owned(), context)
    }

    const fn team_handle(&self, use_writer: bool) -> &PostgresStore {
        let database = TeamDatabase {
            db: &self.reader,
            writer_db: &self.writer,
        };
        TeamRepository {
            database,
            use_writer,
        }
        .table()
        .store
    }
}

#[async_trait]
impl TeamStore for DatabaseTeamStore {
    async fn find_team(&self, team_id: &str, use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        let table = self.open(self.team_handle(use_writer), AuthReadAdapter::Team, "");
        let request = FindOne {
            filter: RowKey::Team(team_id.to_owned()),
            include: Presence::Value(()),
        };
        let found = table
            .find_unique(request)
            .await
            .map_err(|error| TeamStoreError::Read(Box::new(error)))?;
        found
            .map(|text| parse_data(text.as_bytes()))
            .transpose()
            .map_err(|error| TeamStoreError::Row(Box::new(error)))
    }

    async fn find_by_alias(&self, team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        let table = self.open(self.team_handle(false), AuthReadAdapter::Team, "");
        let request = FindMany {
            filter: Presence::Value(team_alias.to_owned()),
            include: Presence::Value(()),
            take: Presence::Omitted,
        };
        let found = table
            .find_many(request)
            .await
            .map_err(|error| TeamStoreError::Read(Box::new(error)))?;
        found
            .iter()
            .map(|text| parse_data(text.as_bytes()))
            .collect::<Result<Vec<JsonData>, _>>()
            .map_err(|error| TeamStoreError::Row(Box::new(error)))
    }

    async fn find_access_group(
        &self,
        access_group_id: &str,
        use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        let client = PrismaClient {
            writer: &self.writer,
            reader: &self.reader,
            availability: self.availability.as_ref(),
        };
        let base = PrismaTableRepository::new(
            Some(client),
            redis_config_change_publisher(None),
            AccessGroupTable,
        );
        let repository = if use_writer { base.with_writer() } else { base };
        let access = repository
            .access()
            .map_err(|error| TeamStoreError::Read(Box::new(AuthReadError::Repository(error))))?;
        let table = self.open(access.read, AuthReadAdapter::Dictable, "access_group");
        let request = FindOne {
            filter: RowKey::AccessGroup(access_group_id.to_owned()),
            include: Presence::Omitted,
        };
        let found = table
            .find_unique(request)
            .await
            .map_err(|error| TeamStoreError::Read(Box::new(error)))?;
        found
            .map(|text| parse_data(text.as_bytes()))
            .transpose()
            .map_err(|error| TeamStoreError::Row(Box::new(error)))
    }
}
