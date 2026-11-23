#![cfg(test)]

use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

const INSERT_USER_SQL: &str = r#"INSERT INTO "LiteLLM_UserTable" ("user_id") VALUES ($1)"#;

const SELECT_INVITATION_SQL: &str = r#"SELECT "id", "user_id", "is_accepted", "accepted_at", "expires_at", "created_at", "created_by", "updated_at", "updated_by" FROM "LiteLLM_InvitationLink" WHERE "id" = $1"#;

const PROBE_INSERT_SQL: &str = r#"INSERT INTO "LiteLLM_InvitationLink" ("id") VALUES ('probe')"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct StoredInvitation {
    pub(crate) id: String,
    pub(crate) user_id: String,
    pub(crate) is_accepted: bool,
    pub(crate) accepted_at: Option<NaiveDateTime>,
    pub(crate) expires_at: NaiveDateTime,
    pub(crate) created_at: NaiveDateTime,
    pub(crate) created_by: String,
    pub(crate) updated_at: NaiveDateTime,
    pub(crate) updated_by: String,
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    pool: Pool,
}

impl Database {
    pub(crate) async fn start() -> Self {
        let url = Environment::parse_from(["user-invitation-tests"])
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

    pub(crate) fn store(&self) -> PostgresStore {
        PostgresStore {
            pool: Some(self.pool.clone()),
        }
    }

    pub(crate) async fn run(&self, sql: &str) {
        self.pool
            .get()
            .await
            .expect("a fixture connection")
            .batch_execute(sql)
            .await
            .expect("the statements run");
    }

    pub(crate) async fn insert_users(&self, users: &[&str]) {
        let store = self.store();
        let client = store.session().await.expect("a fixture session");
        for user in users {
            let parameters: [&(dyn ToSql + Sync); 1] = [user];
            client
                .execute(INSERT_USER_SQL, &parameters)
                .await
                .expect("the user row inserts");
        }
    }

    pub(crate) async fn invitation_count(&self) -> i64 {
        self.pool
            .get()
            .await
            .expect("a fixture connection")
            .query_one(r#"SELECT count(*) FROM "LiteLLM_InvitationLink""#, &[])
            .await
            .expect("the count runs")
            .get(0)
    }

    pub(crate) async fn probe_insert_error_message(&self) -> String {
        let error = self
            .pool
            .get()
            .await
            .expect("a fixture connection")
            .execute(PROBE_INSERT_SQL, &[])
            .await
            .expect_err("the table is gone");
        error
            .as_db_error()
            .expect("a database error")
            .message()
            .to_owned()
    }

    pub(crate) async fn stored_invitation(&self, id: &str) -> Option<StoredInvitation> {
        let found = self
            .pool
            .get()
            .await
            .expect("a fixture connection")
            .query_opt(SELECT_INVITATION_SQL, &[&id])
            .await
            .expect("the invitation row query runs");
        found.map(|row| StoredInvitation {
            id: row.try_get("id").expect("an id"),
            user_id: row.try_get("user_id").expect("a user id"),
            is_accepted: row.try_get("is_accepted").expect("an acceptance flag"),
            accepted_at: row.try_get("accepted_at").expect("an acceptance time"),
            expires_at: row.try_get("expires_at").expect("an expiry"),
            created_at: row.try_get("created_at").expect("a creation time"),
            created_by: row.try_get("created_by").expect("a creator"),
            updated_at: row.try_get("updated_at").expect("an update time"),
            updated_by: row.try_get("updated_by").expect("an updater"),
        })
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}
