use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::key_query_plan::sql::SqlValue;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct Fixture {
    postgres: PostgresFixture,
    store: PostgresStore,
}

impl Fixture {
    pub(super) async fn user_ids(&self, sql: &str, params: &[SqlValue]) -> Vec<String> {
        let bound: Vec<&(dyn ToSql + Sync)> = params.iter().map(SqlValue::bind).collect();
        let client = self.store.session().await.unwrap();
        let rows = client.query(sql, &bound).await.unwrap();
        rows.iter().map(|row| row.get(0)).collect()
    }

    pub(super) async fn close(self) {
        if let Some(pool) = self.store.pool.as_ref() {
            pool.close();
        }
        self.postgres.close_async().await.unwrap();
    }
}

pub(super) async fn database(seed: &str) -> Fixture {
    let url = Environment::parse_from(["user-search-predicate-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).unwrap();
    let postgres = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(postgres.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(4).build().unwrap();
    if !seed.is_empty() {
        pool.get().await.unwrap().batch_execute(seed).await.unwrap();
    }
    Fixture {
        postgres,
        store: PostgresStore { pool: Some(pool) },
    }
}

pub(super) fn user_insert(user_id: &str, email: Option<&str>) -> String {
    let email = email.map_or_else(|| String::from("NULL"), |value| format!("'{value}'"));
    format!(
        "INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email) VALUES ('{user_id}', {email});"
    )
}

pub(super) fn organization_insert(organization_id: &str) -> String {
    let budget_id = format!("budget-{organization_id}");
    format!(
        "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id, created_by, updated_by) VALUES ('{budget_id}', 'test', 'test');INSERT INTO \"LiteLLM_OrganizationTable\" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('{organization_id}', '{organization_id}', '{budget_id}', 'test', 'test');"
    )
}

pub(super) fn membership_insert(user_id: &str, organization_id: &str) -> String {
    format!(
        "INSERT INTO \"LiteLLM_OrganizationMembership\" (user_id, organization_id, user_role) VALUES ('{user_id}', '{organization_id}', 'internal_user');"
    )
}
