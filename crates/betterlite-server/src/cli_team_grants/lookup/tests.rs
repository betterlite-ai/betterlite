#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::fetch_cli_sso_team_details;
use crate::cli_team_grants::records::CliSsoTeamDetail;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Database {
    fixture: PostgresFixture,
    pool: Pool,
}

impl Database {
    async fn start() -> Self {
        let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the catalogs are UTF-8");
        Self::provision(statements).await
    }

    async fn empty() -> Self {
        Self::provision(Vec::new()).await
    }

    async fn provision(statements: Vec<String>) -> Self {
        let url = Environment::parse_from(["cli-team-grant-tests"])
            .url
            .map(SecretString::from);
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

    fn store(&self) -> PostgresStore {
        PostgresStore {
            pool: Some(self.pool.clone()),
        }
    }

    async fn run(&self, sql: &str) {
        let client = self.pool.get().await.expect("a fixture connection");
        client.batch_execute(sql).await.expect("the statements run");
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

// rows of both teams and model tables, one per alias shape the lookup reads
const TEAMS: &str = r#"
INSERT INTO "LiteLLM_ModelTable" (id, aliases, created_by, updated_by) VALUES
    (1, to_jsonb('{"gpt-4": "gpt-4o"}'::text), 'tester', 'tester'),
    (2, '{"gpt-4": "gpt-4o"}'::jsonb, 'tester', 'tester'),
    (3, NULL, 'tester', 'tester'),
    (4, 'null'::jsonb, 'tester', 'tester'),
    (5, '{"gpt-4": 1}'::jsonb, 'tester', 'tester'),
    (6, '[1, 2]'::jsonb, 'tester', 'tester');

INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, models, model_id) VALUES
    ('team-text', 'Text Aliases', ARRAY['gpt-4'], 1),
    ('team-map', NULL, ARRAY['gpt-4', 'claude'], 2),
    ('team-null-alias', 'Null Alias', ARRAY[]::text[], 3),
    ('team-json-null', 'JSON Null', ARRAY['gpt-4'], 4),
    ('team-number', 'Number Alias', ARRAY['gpt-4'], 5),
    ('team-list', 'List Alias', ARRAY['gpt-4'], 6),
    ('team-no-model', 'No Model', ARRAY['gpt-4'], NULL);
"#;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(alias, model)| (String::from(*alias), String::from(*model)))
        .collect()
}

fn detail(
    team_id: &str,
    team_alias: Option<&str>,
    models: &[&str],
    aliases: Option<BTreeMap<String, String>>,
) -> CliSsoTeamDetail {
    CliSsoTeamDetail {
        team_id: Some(String::from(team_id)),
        team_alias: team_alias.map(String::from),
        team_models: strings(models),
        team_model_aliases: aliases,
    }
}

async fn fetch(database: &Database, teams: &[&str]) -> Option<Vec<CliSsoTeamDetail>> {
    fetch_cli_sso_team_details(&database.store(), &strings(teams)).await
}

fn sorted(mut details: Vec<CliSsoTeamDetail>) -> Vec<CliSsoTeamDetail> {
    details.sort_by_key(|detail| detail.team_id.clone());
    details
}

#[tokio::test]
async fn reads_alias_text_into_a_map() {
    let database = Database::start().await;
    database.run(TEAMS).await;
    let details = fetch(&database, &["team-text"]).await;
    database.close().await;
    assert_eq!(
        details,
        Some(vec![detail(
            "team-text",
            Some("Text Aliases"),
            &["gpt-4"],
            Some(map(&[("gpt-4", "gpt-4o")]))
        )])
    );
}

#[tokio::test]
async fn reads_a_stored_alias_map() {
    let database = Database::start().await;
    database.run(TEAMS).await;
    let details = fetch(&database, &["team-map"]).await;
    database.close().await;
    assert_eq!(
        details,
        Some(vec![detail(
            "team-map",
            None,
            &["gpt-4", "claude"],
            Some(map(&[("gpt-4", "gpt-4o")]))
        )])
    );
}

#[tokio::test]
async fn reads_a_sql_null_and_a_json_null_as_no_aliases() {
    let database = Database::start().await;
    database.run(TEAMS).await;
    let details = fetch(
        &database,
        &["team-null-alias", "team-json-null", "team-no-model"],
    )
    .await;
    database.close().await;
    assert_eq!(
        sorted(details.expect("the lookup answers")),
        vec![
            detail("team-json-null", Some("JSON Null"), &["gpt-4"], None),
            detail("team-no-model", Some("No Model"), &["gpt-4"], None),
            detail("team-null-alias", Some("Null Alias"), &[], None),
        ]
    );
}

#[tokio::test]
async fn leaves_out_a_team_id_with_no_row() {
    let database = Database::start().await;
    database.run(TEAMS).await;
    let details = fetch(&database, &["team-text", "team-missing"]).await;
    database.close().await;
    assert_eq!(
        details,
        Some(vec![detail(
            "team-text",
            Some("Text Aliases"),
            &["gpt-4"],
            Some(map(&[("gpt-4", "gpt-4o")]))
        )])
    );
}

#[tokio::test]
async fn an_empty_team_list_answers_no_teams_without_a_session() {
    let store = PostgresStore { pool: None };
    assert_eq!(
        fetch_cli_sso_team_details(&store, &[]).await,
        Some(Vec::new())
    );
}

#[tokio::test]
async fn an_alias_map_with_a_number_gives_none() {
    let database = Database::start().await;
    database.run(TEAMS).await;
    let details = fetch(&database, &["team-number"]).await;
    database.close().await;
    assert_eq!(details, None);
}

#[tokio::test]
async fn an_alias_column_that_is_not_text_or_a_map_gives_none() {
    let database = Database::start().await;
    database.run(TEAMS).await;
    let details = fetch(&database, &["team-list"]).await;
    database.close().await;
    assert_eq!(details, None);
}

#[tokio::test]
async fn a_failed_query_gives_none() {
    let database = Database::empty().await;
    let details = fetch(&database, &["team-text"]).await;
    database.close().await;
    assert_eq!(details, None);
}
