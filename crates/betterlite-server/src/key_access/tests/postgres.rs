use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamRepository};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};

use crate::key_access::error::KeyAccessError;
use crate::key_access::service_account::{TeamDirectory, validate_service_account_team};

const TEAMS: &str = r#"CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT NOT NULL PRIMARY KEY,
    members_with_roles JSONB NOT NULL DEFAULT '[]'
);
INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('team-1');"#;

#[derive(Parser)]
struct Database {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

#[tokio::test]
async fn team_directory_reads_the_table() {
    let url = Database::parse_from(["key-access-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![TEAMS.to_owned()])
        .resolve()
        .await
        .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let store = PostgresStore {
        pool: Some(
            Pool::builder(Manager::new(config, NoTls))
                .max_size(1)
                .build()
                .unwrap(),
        ),
    };
    let repository = TeamRepository {
        database: TeamDatabase {
            db: &store,
            writer_db: &store,
        },
        use_writer: false,
    };
    assert!(repository.team_exists("team-1").await.unwrap());
    assert!(!repository.team_exists("team-2").await.unwrap());
    let teams: Option<&dyn TeamDirectory> = Some(&repository);
    validate_service_account_team(Some("team-1"), teams)
        .await
        .unwrap();
    let unknown = validate_service_account_team(Some("team-2"), teams)
        .await
        .unwrap_err();
    assert!(matches!(unknown, KeyAccessError::ServiceTeamUnknown { .. }));
}
