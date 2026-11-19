use std::env::split_paths;
use std::ffi::OsString;

use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

use crate::connection::{PostgresFixture, RandomDatabaseName};
use crate::seed::{DatabaseFixture, FixtureSeed};

pub(super) const SAMPLE: &str = r#"{
  "users": [
    {"user_id": "alice", "user_email": "alice@example.com"},
    {"user_id": "bob", "user_email": "bob@example.com"},
    {"user_id": "carol"}
  ],
  "keys": [
    {"token": "sk-alice-token", "user_id": "alice", "team_id": "team-red", "role": "proxy_admin"},
    {"token": "sk-bob-token", "user_id": "bob", "team_id": "team-blue"},
    {"token": "sk-carol-token", "user_id": "carol"}
  ],
  "teams": [
    {"team_id": "team-red", "team_alias": "Red Team"},
    {"team_id": "team-blue"}
  ],
  "scim": {
    "users": [
      {
        "user_id": "alice",
        "created_at": "2025-01-01T00:00:00",
        "updated_at": "2025-01-02T00:00:00",
        "user_alias": "Alice",
        "sso_user_id": "",
        "user_role": "proxy_admin",
        "teams": ["team-red", "team-unseeded"],
        "metadata": {"origin": "scim", "scim_managed": true}
      },
      {
        "user_id": "bob",
        "created_at": "2025-02-01T00:00:00",
        "updated_at": "2025-02-02T00:00:00"
      }
    ],
    "teams": [
      {
        "team_id": "team-red",
        "created_at": "2025-03-01T00:00:00",
        "updated_at": "2025-03-02T00:00:00",
        "members_with_roles": [
          {"user_id": "alice", "role": "admin"},
          {"user_email": "roster@example.com", "role": "user"}
        ],
        "members": ["alice"],
        "metadata": {"origin": "scim"}
      },
      {"team_id": "team-blue"}
    ],
    "keys": [
      {"token": "sk-alice-token", "blocked": true, "metadata": {"scim_blocked": true}},
      {"token": "sk-bob-token", "blocked": false},
      {"token": "sk-carol-token"}
    ]
  }
}"#;

pub(super) fn decode(seed: &str) -> FixtureSeed {
    let document = format!(r#"{{"seed":{seed}}}"#);
    serde_json::from_str::<DatabaseFixture>(&document)
        .expect("the scim seed case decodes")
        .seed
}

pub(super) fn sample_seed() -> FixtureSeed {
    decode(SAMPLE)
}

#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    postgres_test_url: Option<String>,
    #[arg(long, env = "PATH")]
    path: Option<OsString>,
}

pub(super) fn postgres_available() -> bool {
    let environment = TestEnvironment::parse_from(["scim-fixture-tests"]);
    environment.postgres_test_url.is_some()
        || environment.path.is_some_and(|path| {
            split_paths(&path).any(|directory| directory.join("docker").is_file())
        })
}

pub(super) async fn provision(admin_url: &SecretString, seed: &FixtureSeed) -> PostgresFixture {
    PostgresFixture::provision_with_name_generator(Some(admin_url), seed, &mut RandomDatabaseName)
        .resolve()
        .await
        .unwrap()
}

pub(super) async fn connect(url: &SecretString) -> (Client, JoinSet<()>) {
    let (client, connection) = tokio_postgres::connect(url.expose_secret(), NoTls)
        .await
        .unwrap();
    let mut driver = JoinSet::new();
    driver.spawn(async move {
        connection.await.unwrap();
    });
    (client, driver)
}

pub(super) async fn disconnect(client: Client, mut driver: JoinSet<()>) {
    drop(client);
    driver.join_next().await.unwrap().unwrap();
}
