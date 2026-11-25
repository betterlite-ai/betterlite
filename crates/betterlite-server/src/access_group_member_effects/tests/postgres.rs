#![cfg(test)]

use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use deadpool_postgres::{Client, GenericClient, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::access_group_read::protocols::error::AccessGroupStoreError;
use crate::access_group_read::protocols::postgres::PgAccessGroupTables;
use crate::access_group_read::protocols::records::KeyRecord;
use crate::access_group_read::protocols::tables::{
    AccessGroupTable, AccessGroupTx, AgentTable, KeyTable, TeamTable,
};

const SEEDED_AT: &str = "2000-01-01 00:00:00";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct Database {
    fixture: PostgresFixture,
    pool: Pool,
}

impl Database {
    pub(super) async fn start() -> Self {
        let url = Environment::parse_from(["access-group-member-effects-tests"])
            .url
            .map(SecretString::from);
        let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the catalogs are UTF-8");
        let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
            .resolve()
            .await
            .expect("a Postgres database fixture");
        let config = fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses");
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(2)
            .build()
            .expect("the fixture pool builds");
        Self { fixture, pool }
    }

    pub(super) async fn client(&self) -> Client {
        self.pool.get().await.expect("a fixture connection")
    }

    pub(super) async fn execute(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
        self.client()
            .await
            .execute(statement, parameters)
            .await
            .expect("the fixture statement");
    }

    pub(super) async fn query_one(
        &self,
        statement: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Row {
        self.client()
            .await
            .query_one(statement, parameters)
            .await
            .expect("the fixture query returns a row")
    }

    pub(super) async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

pub(super) fn seeded_at() -> NaiveDateTime {
    NaiveDateTime::parse_from_str(SEEDED_AT, "%Y-%m-%d %H:%M:%S")
        .expect("the seed timestamp parses")
}

pub(super) async fn insert_team(database: &Database, team_id: &str, groups: Option<&[String]>) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&team_id, &groups];
    let statement = r#"INSERT INTO "LiteLLM_TeamTable" ("team_id", "team_alias", "access_group_ids") VALUES ($1, $1, $2)"#;
    database.execute(statement, &parameters).await;
}

pub(super) async fn insert_key(database: &Database, token: &str, groups: Option<&[String]>) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&token, &groups];
    let statement =
        r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "access_group_ids") VALUES ($1, $2)"#;
    database.execute(statement, &parameters).await;
}

pub(super) async fn insert_agent(database: &Database, agent_id: &str, groups: Option<&[String]>) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&agent_id, &groups];
    let statement = r#"INSERT INTO "LiteLLM_AgentsTable" ("agent_id", "agent_name", "agent_card_params", "created_by", "updated_by", "access_group_ids") VALUES ($1, $1, '{}', 'seed', 'seed', $2)"#;
    database.execute(statement, &parameters).await;
}

pub(super) async fn age_row(database: &Database, table: &str, key_column: &str, key: &str) {
    let statement = format!(
        r#"UPDATE "{table}" SET "updated_at" = TIMESTAMP '{SEEDED_AT}' WHERE "{key_column}" = $1"#
    );
    let parameters: [&(dyn ToSql + Sync); 1] = [&key];
    database.execute(&statement, &parameters).await;
}

pub(super) async fn updated_at(
    database: &Database,
    table: &str,
    key_column: &str,
    key: &str,
) -> NaiveDateTime {
    let statement = format!(r#"SELECT "updated_at" FROM "{table}" WHERE "{key_column}" = $1"#);
    let parameters: [&(dyn ToSql + Sync); 1] = [&key];
    let row = database.query_one(&statement, &parameters).await;
    row.get(0)
}

pub(super) async fn team_groups(database: &Database, team_id: &str) -> Option<Vec<String>> {
    let statement = r#"SELECT "access_group_ids" FROM "LiteLLM_TeamTable" WHERE "team_id" = $1"#;
    let parameters: [&(dyn ToSql + Sync); 1] = [&team_id];
    let row = database.query_one(statement, &parameters).await;
    row.get(0)
}

pub(super) async fn key_groups(database: &Database, token: &str) -> Option<Vec<String>> {
    let statement =
        r#"SELECT "access_group_ids" FROM "LiteLLM_VerificationToken" WHERE "token" = $1"#;
    let parameters: [&(dyn ToSql + Sync); 1] = [&token];
    let row = database.query_one(statement, &parameters).await;
    row.get(0)
}

pub(super) async fn agent_groups(database: &Database, agent_id: &str) -> Option<Vec<String>> {
    let statement = r#"SELECT "access_group_ids" FROM "LiteLLM_AgentsTable" WHERE "agent_id" = $1"#;
    let parameters: [&(dyn ToSql + Sync); 1] = [&agent_id];
    let row = database.query_one(statement, &parameters).await;
    row.get(0)
}

pub(super) async fn team_exists(database: &Database, team_id: &str) -> bool {
    let statement = r#"SELECT COUNT(*) FROM "LiteLLM_TeamTable" WHERE "team_id" = $1"#;
    let parameters: [&(dyn ToSql + Sync); 1] = [&team_id];
    let row = database.query_one(statement, &parameters).await;
    row.get::<_, i64>(0) > 0
}

pub(super) struct RejectingTx<'db, C> {
    pub(super) inner: PgAccessGroupTables<'db, C>,
    pub(super) reject: &'db str,
}

impl<C: GenericClient + Sync> AccessGroupTx for RejectingTx<'_, C> {
    fn litellm_accessgrouptable(&self) -> impl AccessGroupTable {
        self.inner.litellm_accessgrouptable()
    }

    fn litellm_teamtable(&self) -> impl TeamTable {
        self.inner.litellm_teamtable()
    }

    fn litellm_verificationtoken(&self) -> impl KeyTable {
        RejectingKeys {
            inner: self.inner.litellm_verificationtoken(),
            reject: self.reject,
        }
    }

    fn litellm_agentstable(&self) -> impl AgentTable {
        self.inner.litellm_agentstable()
    }
}

struct RejectingKeys<'reject, K: KeyTable> {
    inner: K,
    reject: &'reject str,
}

impl<K: KeyTable> KeyTable for RejectingKeys<'_, K> {
    async fn find(&self, token: &str) -> Result<Option<KeyRecord>, AccessGroupStoreError> {
        self.inner.find(token).await
    }

    async fn find_carrying(
        &self,
        access_group_id: &str,
    ) -> Result<Vec<KeyRecord>, AccessGroupStoreError> {
        self.inner.find_carrying(access_group_id).await
    }

    async fn set_access_group_ids(
        &self,
        token: &str,
        ids: &[String],
    ) -> Result<(), AccessGroupStoreError> {
        if token == self.reject {
            return Err(AccessGroupStoreError::RecordNotFound);
        }
        self.inner.set_access_group_ids(token, ids).await
    }
}
