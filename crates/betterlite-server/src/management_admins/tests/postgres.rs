use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};

use super::support::{FakeUsers, caller, user};
use crate::management_admins::adapters::PostgresTeamReads;
use crate::management_admins::privileges::admin_can_invite_user;

// t1 and t2 hold other users' rosters, t3 has none, t4 repeats u1's roster, t5 names u1 by email only
const STORAGE: &str = r#"CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT NOT NULL,
    members_with_roles JSONB NOT NULL DEFAULT '{}',
    CONSTRAINT "LiteLLM_TeamTable_pkey" PRIMARY KEY (team_id)
);
INSERT INTO "LiteLLM_TeamTable" (team_id, members_with_roles) VALUES
    ('t1', '[{"user_id":"u1","role":"admin"}]'::JSONB),
    ('t2', '[{"user_id":"u2","role":"admin"}]'::JSONB),
    ('t3', '{}'::JSONB),
    ('t4', '[{"user_id":"u1","role":"admin"}]'::JSONB),
    ('t5', '[{"user_email":"u1","role":"admin"}]'::JSONB);"#;

const FINGERPRINT_SQL: &str = r#"SELECT COALESCE(md5(string_agg(team_id || members_with_roles::TEXT, ',' ORDER BY team_id)), '') FROM "LiteLLM_TeamTable""#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn read_teams() -> (PostgresFixture, Pool, PostgresTeamReads) {
    let url = Environment::parse_from(["management-admins-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![STORAGE.to_owned()])
            .resolve()
            .await
            .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(2)
        .build()
        .unwrap();
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    (fixture, pool, PostgresTeamReads { store })
}

async fn fingerprint(pool: &Pool) -> String {
    let client = pool.get().await.unwrap();
    client.query_one(FINGERPRINT_SQL, &[]).await.unwrap().get(0)
}

pub(super) async fn team_query_exact_ids() {
    let (fixture, pool, teams) = read_teams().await;
    let before = fingerprint(&pool).await;
    let admin = caller(Some("u1"));
    let nested = FakeUsers::new(vec![
        ("u1", user("u1", &["t1", "t5"])),
        ("u2", user("u2", &["t4"])),
    ]);
    // t4 repeats the caller's roster but is not one of the caller's teams, so only exact ids count
    assert!(!admin_can_invite_user("u2", &admin, &nested, Some(&teams), None).await);
    let shared = FakeUsers::new(vec![
        ("u1", user("u1", &["t1", "t5"])),
        ("u2", user("u2", &["t1"])),
    ]);
    assert!(admin_can_invite_user("u2", &admin, &shared, Some(&teams), None).await);
    let by_email = FakeUsers::new(vec![
        ("u1", user("u1", &["t5"])),
        ("u2", user("u2", &["t5"])),
    ]);
    assert!(!admin_can_invite_user("u2", &admin, &by_email, Some(&teams), None).await);
    let member = caller(Some("u2"));
    let same_team = FakeUsers::new(vec![
        ("u1", user("u1", &["t1"])),
        ("u2", user("u2", &["t1"])),
    ]);
    assert!(!admin_can_invite_user("u2", &member, &same_team, Some(&teams), None).await);
    let after = fingerprint(&pool).await;
    assert_eq!(before, after);
    pool.close();
    fixture.close_async().await.unwrap();
}
