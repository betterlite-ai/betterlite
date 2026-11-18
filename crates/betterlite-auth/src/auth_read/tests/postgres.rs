#![cfg(test)]

use std::convert::Infallible;
use std::sync::Arc;

use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::rows::KeyColumn;
use betterlite_db::user_repository::tables::USER_TABLE;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::auth_read::include::UserInclude;
use crate::auth_read::repository::{NativeKey, RepositoryTable};
use crate::auth_read::table::{AuthTable, FindMany, FindOne, Presence};

const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by)
    VALUES ('budget-1', 'tester', 'tester');
INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by)
    VALUES ('org-1', 'org one', 'budget-1', 'tester', 'tester');
INSERT INTO "LiteLLM_UserTable" (user_id, user_email)
    VALUES ('user-1', 'first@example.com'), ('user-2', 'second@example.com');
INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id, user_role, spend)
    VALUES ('user-1', 'org-1', 'admin', 0.0);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["auth-read-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let pool = Pool::builder(Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    ))
    .max_size(2)
    .build()
    .expect("the database pool");
    (fixture, pool)
}

async fn seed(pool: &Pool) {
    let client = pool.get().await.expect("the database connection");
    client.batch_execute(SEED).await.expect("the seeded rows");
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

fn key(user_id: &str) -> NativeKey<Infallible> {
    NativeKey {
        column: KeyColumn::Id,
        value: Box::new(user_id.to_owned()),
    }
}

#[tokio::test]
async fn native_user_reads_select_requested_memberships() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    {
        let client = pool.get().await.expect("the database connection");
        let repository: RepositoryTable<Client, _> = RepositoryTable {
            client: Some(Arc::new(client)),
            table: USER_TABLE,
        };
        let include = Presence::Value(UserInclude {
            organization_memberships: true,
        });
        let found = repository
            .find_unique(FindOne {
                filter: key("user-1"),
                include: include.clone(),
            })
            .await
            .expect("the user read")
            .expect("the stored user");
        let memberships = found
            .organization_memberships
            .expect("the included memberships");
        assert_eq!(memberships.len(), 1);
        assert_eq!(memberships[0].organization_id, "org-1");
        assert_eq!(memberships[0].user_role.as_deref(), Some("admin"));
        let listed = repository
            .find_many(FindMany {
                filter: Presence::Omitted,
                include: include.clone(),
                take: Presence::Omitted,
            })
            .await
            .expect("the user list");
        assert_eq!(listed.len(), 2);
        let first = listed
            .iter()
            .find(|user| user.user_id == "user-1")
            .expect("the seeded user");
        assert_eq!(
            first
                .organization_memberships
                .as_ref()
                .expect("the included memberships")
                .len(),
            1
        );
        let second = listed
            .iter()
            .find(|user| user.user_id == "user-2")
            .expect("the seeded user");
        assert_eq!(
            second
                .organization_memberships
                .as_ref()
                .expect("the empty membership list")
                .len(),
            0
        );
        let plain = repository
            .find_unique(FindOne {
                filter: key("user-2"),
                include: Presence::Omitted,
            })
            .await
            .expect("the plain read")
            .expect("the stored user");
        assert!(plain.organization_memberships.is_none());
    }
    close(fixture, pool).await;
}

#[tokio::test]
async fn native_first_returns_one_of_several_rows() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    {
        let client = pool.get().await.expect("the database connection");
        let repository: RepositoryTable<Client, _> = RepositoryTable {
            client: Some(Arc::new(client)),
            table: USER_TABLE,
        };
        let first = repository
            .find_first(FindOne {
                filter: Vec::new(),
                include: Presence::Omitted,
            })
            .await
            .expect("the filtered read does not fail on several matches")
            .expect("one of the stored users");
        assert!(["user-1", "user-2"].contains(&first.user_id.as_str()));
        let unique = repository
            .find_unique(FindOne {
                filter: key("user-2"),
                include: Presence::Omitted,
            })
            .await
            .expect("the keyed read")
            .expect("the stored user");
        assert_eq!(unique.user_id, "user-2");
    }
    close(fixture, pool).await;
}
