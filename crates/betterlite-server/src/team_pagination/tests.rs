#![cfg(test)]

use std::io;
use std::panic::PoisonError;
use std::str;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::get_paginated_teams;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Fixture {
    postgres: PostgresFixture,
    store: PostgresStore,
    availability: Availability,
}

impl Fixture {
    const fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    async fn database(seed: &str) -> Self {
        let url = Environment::parse_from(["team-pagination-tests"])
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
        Self {
            postgres,
            store: PostgresStore { pool: Some(pool) },
            availability: Availability::default(),
        }
    }

    async fn close(self) {
        if let Some(pool) = self.store.pool.as_ref() {
            pool.close();
        }
        self.postgres.close_async().await.unwrap();
    }
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes).unwrap().to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

// a subscriber that keeps only error events, so a line it captures was logged at error level
fn capture_error_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::ERROR)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

fn team_insert(team_id: &str, team_alias: Option<&str>, roster: &str) -> String {
    let alias = team_alias.map_or_else(|| String::from("NULL"), |value| format!("'{value}'"));
    format!(
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, members_with_roles) VALUES ('{team_id}', {alias}, '{roster}'::jsonb);"#
    )
}

fn seed(teams: &[(&str, Option<&str>)]) -> String {
    teams
        .iter()
        .map(|(team_id, team_alias)| team_insert(team_id, *team_alias, "[]"))
        .collect()
}

fn alias_names(teams: &[LiteLLMTeamTable]) -> Vec<Option<&str>> {
    teams
        .iter()
        .map(|team| team.fields.base.team_alias.as_deref())
        .collect()
}

#[tokio::test]
async fn pages_ascend_by_alias_with_the_full_count() {
    let fixture = Fixture::database(&seed(&[
        ("t-charlie", Some("Charlie")),
        ("t-alpha", Some("Alpha")),
        ("t-echo", Some("Echo")),
        ("t-bravo", Some("Bravo")),
        ("t-delta", Some("Delta")),
    ]))
    .await;
    let (first, total) = get_paginated_teams(fixture.client(), 2, 1).await;
    assert_eq!(alias_names(&first), ["Alpha", "Bravo"].map(Some));
    assert_eq!(total, 5);
    let (second, total) = get_paginated_teams(fixture.client(), 2, 2).await;
    assert_eq!(alias_names(&second), ["Charlie", "Delta"].map(Some));
    assert_eq!(total, 5);
    let (third, total) = get_paginated_teams(fixture.client(), 2, 3).await;
    assert_eq!(alias_names(&third), ["Echo"].map(Some));
    assert_eq!(total, 5);
    let (fourth, total) = get_paginated_teams(fixture.client(), 2, 4).await;
    assert!(fourth.is_empty());
    assert_eq!(total, 5);
    fixture.close().await;
}

#[tokio::test]
async fn null_alias_sorts_after_named_teams() {
    let fixture = Fixture::database(&seed(&[
        ("t-zulu", Some("Zulu")),
        ("t-nameless", None),
        ("t-alpha", Some("Alpha")),
    ]))
    .await;
    let (first, total) = get_paginated_teams(fixture.client(), 2, 1).await;
    assert_eq!(alias_names(&first), ["Alpha", "Zulu"].map(Some));
    assert_eq!(total, 3);
    let (second, _) = get_paginated_teams(fixture.client(), 2, 2).await;
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].fields.base.team_id, "t-nameless");
    assert!(second[0].fields.base.team_alias.is_none());
    fixture.close().await;
}

#[tokio::test]
async fn zero_page_size_returns_only_the_count() {
    let fixture = Fixture::database(&seed(&[
        ("t-one", Some("One")),
        ("t-two", Some("Two")),
        ("t-three", Some("Three")),
    ]))
    .await;
    let (teams, total) = get_paginated_teams(fixture.client(), 0, 1).await;
    assert!(teams.is_empty());
    assert_eq!(total, 3);
    fixture.close().await;
}

#[tokio::test]
async fn reader_store_serves_rows_and_count_until_unavailable() {
    let reader = Fixture::database(&seed(&[
        ("t-reader-one", Some("Reader One")),
        ("t-reader-two", Some("Reader Two")),
    ]))
    .await;
    let writer = Fixture::database(&seed(&[("t-writer", Some("Writer"))])).await;
    let client = PrismaClient {
        writer: &writer.store,
        reader: &reader.store,
        availability: &reader.availability,
    };
    let (teams, total) = get_paginated_teams(client, 10, 1).await;
    assert_eq!(alias_names(&teams), ["Reader One", "Reader Two"].map(Some));
    assert_eq!(total, 2);
    reader
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let (teams, total) = get_paginated_teams(client, 10, 1).await;
    assert_eq!(alias_names(&teams), ["Writer"].map(Some));
    assert_eq!(total, 1);
    reader.close().await;
    writer.close().await;
}

#[tokio::test]
async fn missing_pool_failure_returns_empty_and_logs() {
    let missing = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &missing,
        reader: &missing,
        availability: &availability,
    };
    let (logs, _guard) = capture_error_logs();
    let (teams, total) = get_paginated_teams(client, 10, 1).await;
    assert!(teams.is_empty());
    assert_eq!(total, 0);
    assert!(
        logs.contents()
            .contains("[Non-Blocking] Error getting paginated teams:")
    );
}

#[tokio::test]
async fn page_zero_failure_returns_empty_and_logs() {
    let fixture = Fixture::database(&seed(&[("t-one", Some("One"))])).await;
    let (logs, _guard) = capture_error_logs();
    let (teams, total) = get_paginated_teams(fixture.client(), 2, 0).await;
    assert!(teams.is_empty());
    assert_eq!(total, 0);
    let captured = logs.contents();
    assert!(captured.contains("[Non-Blocking] Error getting paginated teams:"));
    assert!(captured.contains("team page 0"));
    fixture.close().await;
}

#[tokio::test]
async fn undecodable_owner_role_failure_returns_empty_and_logs() {
    let fixture = Fixture::database(&team_insert(
        "t-owner",
        Some("Owner"),
        r#"[{"user_id":"u1","role":"owner"}]"#,
    ))
    .await;
    let (logs, _guard) = capture_error_logs();
    let (teams, total) = get_paginated_teams(fixture.client(), 10, 1).await;
    assert!(teams.is_empty());
    assert_eq!(total, 0);
    let captured = logs.contents();
    assert!(captured.contains("[Non-Blocking] Error getting paginated teams:"));
    assert!(captured.contains("cannot be decoded"));
    fixture.close().await;
}
