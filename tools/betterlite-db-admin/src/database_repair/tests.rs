#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use betterlite_db::database_repair::cells::RowError;
use betterlite_db::database_repair::migration::{ImportEvent, ImportFailure, ImportReport};
use betterlite_db::database_repair::unassigned::PRODUCTION_WARNING;
use betterlite_db_admin::command_shell::{
    AdminEnvironment, DbAdminBackend, Diagnostics, ExitStatus, run,
};
use betterlite_db_admin::commands::{Operation, VERIFICATION_TOKENS_KEY};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

const SCHEMA_SQL: &str =
    include_str!("../../../../crates/betterlite-db/src/database_repair/fixtures/schema.sql");

const SEED_SQL: &str = r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "team_id", "key_name") VALUES ('tok-one', 'team-a', 'one');
INSERT INTO "LiteLLM_SpendLogs" ("request_id", "call_type", "api_key", "team_id", "startTime", "endTime") VALUES ('req-one', 'completion', 'tok-one', NULL, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);"#;

const SPEND_TEAM_SQL: &str =
    r#"SELECT "team_id" FROM "LiteLLM_SpendLogs" WHERE "request_id" = $1::text"#;

const TOKEN_EXISTS_SQL: &str = r#"SELECT EXISTS (SELECT 1 FROM "LiteLLM_VerificationToken" WHERE "token" = $1::text) AS "present""#;

// the CSV columns LiteLLM's migrate_keys.py requires, in the order it reads them
const COLUMNS: [&str; 31] = [
    "token",
    "key_name",
    "key_alias",
    "soft_budget_cooldown",
    "spend",
    "expires",
    "models",
    "aliases",
    "config",
    "user_id",
    "team_id",
    "permissions",
    "max_parallel_requests",
    "metadata",
    "tpm_limit",
    "rpm_limit",
    "max_budget",
    "budget_duration",
    "budget_reset_at",
    "allowed_cache_controls",
    "model_spend",
    "model_max_budget",
    "budget_id",
    "blocked",
    "created_at",
    "updated_at",
    "allowed_routes",
    "object_permission_id",
    "created_by",
    "updated_by",
    "organization_id",
];

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Postgres {
    fixture: PostgresFixture,
    client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl Postgres {
    async fn new() -> Self {
        let url = Environment::parse_from(["database-repair-admin"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .expect("a test schema");
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        client
            .batch_execute(SCHEMA_SQL)
            .await
            .expect("the pinned LiteLLM schema");
        Self {
            fixture,
            client,
            tasks,
        }
    }

    async fn close(self) {
        let Self {
            fixture,
            client,
            tasks,
        } = self;
        drop(client);
        drop(tasks);
        fixture.close_async().await.expect("the schema is dropped");
    }

    async fn spend_team(&self, request_id: &str) -> Option<String> {
        self.client
            .query_one(SPEND_TEAM_SQL, &[&request_id])
            .await
            .expect("the spend log row")
            .get("team_id")
    }

    async fn token_present(&self, token: &str) -> bool {
        self.client
            .query_one(TOKEN_EXISTS_SQL, &[&token])
            .await
            .expect("the token lookup")
            .get("present")
    }
}

struct Recorder(Vec<String>);

impl Diagnostics for Recorder {
    fn output(&mut self, line: &str) {
        self.0.push(format!("out|{line}"));
    }

    fn error(&mut self, line: &str) {
        self.0.push(format!("err|{line}"));
    }
}

async fn run_command(
    arguments: &[&str],
    database_url: &str,
    backend: &DbAdminBackend,
) -> (ExitStatus, Vec<String>) {
    let environment = AdminEnvironment {
        variables: BTreeMap::from([("DATABASE_URL".to_owned(), database_url.to_owned())]),
        home: None,
    };
    let mut argv = vec![OsString::from("betterlite-db-admin")];
    argv.extend(arguments.iter().map(OsString::from));
    let mut recorder = Recorder(Vec::new());
    let status = Box::pin(run(argv, environment, backend, &mut recorder)).await;
    (status, recorder.0)
}

fn token_csv() -> String {
    let record = COLUMNS
        .iter()
        .map(|name| match *name {
            "token" => "\"tok-csv\"",
            "key_name" => "\"from-csv\"",
            "spend" => "\"0.5\"",
            _ => "\"\"",
        })
        .collect::<Vec<&str>>()
        .join(",");
    format!("{}\n{record}\n", COLUMNS.join(","))
}

#[tokio::test]
async fn dispatches_native_operation() {
    let postgres = Postgres::new().await;
    postgres
        .client
        .batch_execute(SEED_SQL)
        .await
        .expect("the seeded rows");
    let url = postgres.fixture.options.url.expose_secret().to_owned();
    let backend = crate::backend();
    let (status, lines) = run_command(&["update-unassigned-teams"], &url, &backend).await;
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(
        lines,
        vec![
            format!("out|{PRODUCTION_WARNING}"),
            "out|Updated unassigned teams, Response=1".to_owned(),
        ]
    );
    assert_eq!(
        postgres.spend_team("req-one").await.as_deref(),
        Some("team-a")
    );
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("tokens.csv");
    fs::write(&path, token_csv()).expect("the CSV export");
    let (status, lines) = run_command(
        &[
            "migrate-verification-tokens",
            "--csv",
            path.to_str().expect("a UTF-8 path"),
        ],
        &url,
        &backend,
    )
    .await;
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(
        lines,
        vec![
            "out|Successfully migrated token: tok-csv".to_owned(),
            "out|Migration completed!".to_owned(),
            "out|Successfully processed: 1 records".to_owned(),
            "out|Errors encountered: 0 records".to_owned(),
        ]
    );
    assert!(postgres.token_present("tok-csv").await);
    let (status, lines) = run_command(
        &[
            "migrate-verification-tokens",
            "--csv",
            "/nonexistent/tokens.csv",
        ],
        &url,
        &backend,
    )
    .await;
    assert_eq!(status, ExitStatus::Failure);
    assert!(lines.iter().any(|line| line.starts_with("err|")));
    postgres.close().await;
}

#[tokio::test]
async fn rejects_bad_arguments() {
    rejects(&["update-unassigned-teams", "extra"]).await;
    rejects(&["migrate-verification-tokens"]).await;
    rejects(&["migrate-verification-tokens", "--csv"]).await;
    rejects(&[
        "migrate-verification-tokens",
        "--csv",
        "tokens.csv",
        "extra",
    ])
    .await;
}

async fn rejects(arguments: &[&str]) {
    let backend = crate::backend();
    let (status, lines) = run_command(arguments, "postgresql://pinned.example/db", &backend).await;
    assert_eq!(status, ExitStatus::Usage);
    assert_eq!(status.code(), 2);
    assert!(lines.iter().all(|line| line.starts_with("err|")));
}

#[test]
fn keys_the_import_operation_without_the_csv_path() {
    let operation = Operation::MigrateVerificationTokens {
        csv: PathBuf::from("tokens.csv"),
    };
    let command = operation.command(&BTreeMap::new());
    assert_eq!(command.operation(), VERIFICATION_TOKENS_KEY);
}

#[test]
fn formats_the_migration_lines() {
    let report = ImportReport {
        events: vec![
            ImportEvent::Migrated {
                token: "tok-one".to_owned(),
            },
            ImportEvent::Skipped {
                token: "tok-two".to_owned(),
            },
            ImportEvent::Failed {
                token: Some("tok-three".to_owned()),
                source: ImportFailure::Decode(Box::new(RowError::MissingColumn { name: "token" })),
            },
            ImportEvent::Failed {
                token: None,
                source: ImportFailure::Decode(Box::new(RowError::TooDeep { name: "config" })),
            },
        ],
    };
    assert_eq!(
        super::report_lines(&report),
        vec![
            "Successfully migrated token: tok-one".to_owned(),
            "Token tok-two already exists, skipping...".to_owned(),
            "Error processing row with token tok-three: the CSV row has no token value".to_owned(),
            "Error processing row with token unknown: the config value is a JSON document deeper than 64"
                .to_owned(),
            "Migration completed!".to_owned(),
            "Successfully processed: 1 records".to_owned(),
            "Errors encountered: 2 records".to_owned(),
        ]
    );
}
