use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use tokio_util::task::TaskTracker;

use super::ports::{OrgStore, StoreAnswer};
use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::table::{
    AuthReadError, AuthReadTable, AuthTable, Create, FindMany, FindOne, Presence, ReadContext,
    Update,
};

// the row joins its Prisma relations and reads a null model list as the [] Prisma decodes for an empty relation
const ORG_BY_ID_SQL: &str = r#"SELECT (to_jsonb(o) || jsonb_build_object('models', COALESCE(to_jsonb(o.models), '[]'::jsonb)))::text FROM "LiteLLM_OrganizationTable" o WHERE o.organization_id = $1"#;
const ORG_BY_ID_WITH_BUDGET_SQL: &str = r#"SELECT (to_jsonb(o) || jsonb_build_object('models', COALESCE(to_jsonb(o.models), '[]'::jsonb), 'litellm_budget_table', (SELECT to_jsonb(b) || jsonb_build_object('allowed_models', COALESCE(to_jsonb(b.allowed_models), '[]'::jsonb)) FROM "LiteLLM_BudgetTable" b WHERE b.budget_id = o.budget_id)))::text FROM "LiteLLM_OrganizationTable" o WHERE o.organization_id = $1"#;
const ORG_BY_ALIAS_SQL: &str = r#"SELECT (to_jsonb(o) || jsonb_build_object('models', COALESCE(to_jsonb(o.models), '[]'::jsonb)))::text FROM "LiteLLM_OrganizationTable" o WHERE o.organization_alias = $1"#;
const PROJECT_BY_ID_SQL: &str = r#"SELECT (to_jsonb(p) || jsonb_build_object('litellm_budget_table', (SELECT to_jsonb(b) || jsonb_build_object('allowed_models', COALESCE(to_jsonb(b.allowed_models), '[]'::jsonb)) FROM "LiteLLM_BudgetTable" b WHERE b.budget_id = p.budget_id)))::text FROM "LiteLLM_ProjectTable" p WHERE p.project_id = $1"#;

// the lookups read by key and by alias only, so the table's other actions are never wired
const UNREAD: AuthReadError<Infallible> = AuthReadError::Repository(DbError::CreateNull);

// one query over one Postgres handle; the deadline adapter bounds it and rows stay JSON text until decoded
pub struct ReadTable {
    pub store: PostgresStore,
    pub sql: &'static str,
    pub value: String,
}

impl ReadTable {
    async fn rows(&self) -> Result<Vec<String>, AuthReadError<Infallible>> {
        let client = self
            .store
            .session()
            .await
            .map_err(AuthReadError::Repository)?;
        let found = client
            .query(self.sql, &[&self.value])
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
impl AuthTable for ReadTable {
    type Row = String;
    type Key = ();
    type Filter = ();
    type Include = ();
    type Data = ();
    type Failure = Infallible;

    // find_many is the one action this table is read through, so a mutant of the other bodies never runs
    #[cfg_attr(test, mutants::skip)]
    async fn find_unique(
        &self,
        _request: FindOne<(), ()>,
    ) -> Result<Option<String>, AuthReadError<Infallible>> {
        Ok(self.rows().await?.into_iter().next())
    }

    #[cfg_attr(test, mutants::skip)]
    async fn find_first(
        &self,
        request: FindOne<(), ()>,
    ) -> Result<Option<String>, AuthReadError<Infallible>> {
        self.find_unique(request).await
    }

    async fn find_many(
        &self,
        _request: FindMany<(), ()>,
    ) -> Result<Vec<String>, AuthReadError<Infallible>> {
        self.rows().await
    }

    #[cfg_attr(test, mutants::skip)]
    async fn update(
        &self,
        _request: Update<(), ()>,
    ) -> Result<Option<String>, AuthReadError<Infallible>> {
        Err(UNREAD)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn create(&self, _request: Create<(), ()>) -> Result<String, AuthReadError<Infallible>> {
        Err(UNREAD)
    }
}

// OrganizationRepository reads through client.db, which B714 sends to the writer while the reader is down
pub struct DatabaseOrgStore {
    pub reader: PostgresStore,
    pub writer: PostgresStore,
    pub availability: Arc<Availability>,
    pub default_deadline_seconds: f64,
    pub tracker: Arc<DbLookupStallTracker<SystemClock>>,
    pub tasks: TaskTracker,
}

impl DatabaseOrgStore {
    fn read_handle(&self) -> &PostgresStore {
        if self.availability.reader_unavailable.load(Ordering::SeqCst) {
            return &self.writer;
        }
        &self.reader
    }

    async fn read(&self, sql: &'static str, value: &str, label: &str) -> StoreAnswer<Vec<String>> {
        let table = ReadTable {
            store: self.read_handle().clone(),
            sql,
            value: value.to_owned(),
        };
        let context = ReadContext {
            default_deadline_seconds: self.default_deadline_seconds,
            tracker: Arc::clone(&self.tracker),
            tasks: self.tasks.clone(),
        };
        let bounded: AuthReadTable<ReadTable> =
            AuthReadAdapter::ModelDump.open(Arc::new(table), label.to_owned(), context);
        let request = FindMany {
            filter: Presence::Omitted,
            include: Presence::Omitted,
            take: Presence::Omitted,
        };
        bounded.find_many(request).await.map_err(Box::from)
    }
}

#[async_trait]
impl OrgStore for DatabaseOrgStore {
    async fn find_org(&self, org_id: &str, include_budget: bool) -> StoreAnswer<Option<String>> {
        Ok(self
            .read(org_sql(include_budget), org_id, "organization")
            .await?
            .into_iter()
            .next())
    }

    async fn find_by_alias(&self, org_alias: &str) -> StoreAnswer<Vec<String>> {
        self.read(ORG_BY_ALIAS_SQL, org_alias, "organization").await
    }

    async fn find_project(&self, project_id: &str) -> StoreAnswer<Option<String>> {
        self.read(PROJECT_BY_ID_SQL, project_id, "project")
            .await
            .map(|rows| rows.into_iter().next())
    }
}

// Prisma adds the include to the query only when the caller passed one
const fn org_sql(include_budget: bool) -> &'static str {
    if include_budget {
        return ORG_BY_ID_WITH_BUDGET_SQL;
    }
    ORG_BY_ID_SQL
}
