#![cfg(test)]

use std::convert::Infallible;
use std::sync::Arc;

use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::rows::KeyColumn;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::auth_read::include::UserInclude;
use crate::auth_read::jwt_rows::{JwtMappingFilter, JwtRows};
use crate::auth_read::repository::{NativeKey, RepositoryTable};
use crate::auth_read::table::{AuthTable, Create, FindMany, FindOne, Presence};
use crate::auth_read::user_rows::{UserCreate, UserEmailFilter, UserRows};

const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by)
    VALUES ('budget-1', 'tester', 'tester');
INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by)
    VALUES ('org-1', 'org one', 'budget-1', 'tester', 'tester');
INSERT INTO "LiteLLM_UserTable" (user_id, user_email)
    VALUES ('user-1', 'first@example.com'), ('user-2', 'second@example.com');
INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id, user_role, spend)
    VALUES ('user-1', 'org-1', 'admin', 0.0);
INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ('hashed-token');
INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_issuer, jwt_claim_name, jwt_claim_value, token, is_active)
    VALUES ('mapping-1', 'https://issuer.example', 'role', 'admin', 'hashed-token', true);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) async fn database() -> (PostgresFixture, Pool) {
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

pub(super) async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

fn key(user_id: &str) -> NativeKey<Infallible> {
    NativeKey {
        column: KeyColumn::Id,
        value: Box::new(user_id.to_owned()),
    }
}

fn user_create() -> UserCreate {
    UserCreate {
        user_id: "user-3".to_owned(),
        user_alias: Some("third".to_owned()),
        team_id: None,
        sso_user_id: None,
        organization_id: None,
        object_permission_id: None,
        user_role: Some("internal_user".to_owned()),
        user_email: Some("third@example.com".to_owned()),
        teams: Vec::new(),
        models: Vec::new(),
        max_parallel_requests: None,
        tpm_limit: Some(10),
        rpm_limit: None,
        budget_duration: None,
        allowed_cache_controls: Vec::new(),
        policies: Vec::new(),
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
            table: UserRows,
        };
        let include = Presence::Value(UserInclude {
            organization_memberships: true,
        });
        let found = repository
            .find_unique(FindOne {
                filter: key("user-1"),
                include,
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
                include,
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
            table: UserRows,
        };
        let first = repository
            .find_first(FindOne {
                filter: UserEmailFilter::default(),
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

#[tokio::test]
async fn native_email_filter_selects_one_user() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    {
        let client = pool.get().await.expect("the database connection");
        let repository: RepositoryTable<Client, _> = RepositoryTable {
            client: Some(Arc::new(client)),
            table: UserRows,
        };
        let found = repository
            .find_first(FindOne {
                filter: UserEmailFilter {
                    address: Some("FIRST@EXAMPLE.COM".to_owned()),
                    insensitive: true,
                },
                include: Presence::Value(UserInclude {
                    organization_memberships: true,
                }),
            })
            .await
            .expect("the email read")
            .expect("the matching user");
        assert_eq!(found.user_id, "user-1");
        assert_eq!(
            found
                .organization_memberships
                .expect("the included memberships")
                .len(),
            1
        );
        let missing = repository
            .find_first(FindOne {
                filter: UserEmailFilter {
                    address: Some("missing@example.com".to_owned()),
                    insensitive: true,
                },
                include: Presence::Omitted,
            })
            .await
            .expect("the missing read");
        assert!(missing.is_none());
    }
    close(fixture, pool).await;
}

#[tokio::test]
async fn native_membership_list_applies_take() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    {
        let client = pool.get().await.expect("the database connection");
        let repository: RepositoryTable<Client, _> = RepositoryTable {
            client: Some(Arc::new(client)),
            table: UserRows,
        };
        let include = Presence::Value(UserInclude {
            organization_memberships: true,
        });
        let limited = repository
            .find_many(FindMany {
                filter: Presence::Omitted,
                include,
                take: Presence::Value(1),
            })
            .await
            .expect("the limited list");
        assert_eq!(limited.len(), 1);
        assert!(limited[0].organization_memberships.is_some());
        let empty = repository
            .find_many(FindMany {
                filter: Presence::Omitted,
                include,
                take: Presence::Value(0),
            })
            .await
            .expect("a zero take keeps its meaning");
        assert!(empty.is_empty());
    }
    close(fixture, pool).await;
}

#[tokio::test]
async fn native_create_binds_user_and_returns_memberships() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    {
        let client = pool.get().await.expect("the database connection");
        let repository: RepositoryTable<Client, _> = RepositoryTable {
            client: Some(Arc::new(client)),
            table: UserRows,
        };
        let created = repository
            .create(Create {
                data: user_create(),
                include: Presence::Value(UserInclude {
                    organization_memberships: true,
                }),
            })
            .await
            .expect("the created user");
        assert_eq!(created.user_id, "user-3");
        assert_eq!(created.user_email.as_deref(), Some("third@example.com"));
        assert_eq!(created.user_role.as_deref(), Some("internal_user"));
        assert_eq!(created.tpm_limit, Some(10));
        assert_eq!(
            created
                .organization_memberships
                .expect("the empty membership list")
                .len(),
            0
        );
        let stored = repository
            .find_unique(FindOne {
                filter: key("user-3"),
                include: Presence::Omitted,
            })
            .await
            .expect("the stored user read")
            .expect("the stored user");
        assert_eq!(stored.user_email.as_deref(), Some("third@example.com"));
    }
    close(fixture, pool).await;
}

#[tokio::test]
async fn native_jwt_filter_returns_mapping() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    {
        let client = pool.get().await.expect("the database connection");
        let repository: RepositoryTable<Client, _> = RepositoryTable {
            client: Some(Arc::new(client)),
            table: JwtRows,
        };
        let found = repository
            .find_first(FindOne {
                filter: JwtMappingFilter {
                    claim_name: "role".to_owned(),
                    claim_value: "admin".to_owned(),
                    issuer: "https://issuer.example".to_owned(),
                    active: true,
                },
                include: Presence::Omitted,
            })
            .await
            .expect("the mapping read")
            .expect("the matching mapping");
        assert_eq!(found.token, "hashed-token");
        assert_eq!(found.jwt_issuer, "https://issuer.example");
        assert_eq!(found.jwt_claim_name, "role");
        assert_eq!(found.jwt_claim_value, "admin");
        let inactive = repository
            .find_first(FindOne {
                filter: JwtMappingFilter {
                    claim_name: "role".to_owned(),
                    claim_value: "admin".to_owned(),
                    issuer: "https://issuer.example".to_owned(),
                    active: false,
                },
                include: Presence::Omitted,
            })
            .await
            .expect("the inactive read");
        assert!(inactive.is_none());
    }
    close(fixture, pool).await;
}
