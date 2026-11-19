use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_postgres_fixture::connection::PostgresFixture;

use crate::spend::estimates::report::postgres::PostgresSpendReportStore;
use crate::spend::estimates::report::{
    ReportDateError, ReportError, SpendReportSource, TagSpendRow, TeamSpendRow,
    get_spend_report_for_time_range,
};

use super::support::decimal;

struct RecordingSource {
    calls: Mutex<Vec<&'static str>>,
    bounds: Mutex<Vec<(DateTime<Utc>, DateTime<Utc>)>>,
}

impl RecordingSource {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            bounds: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendReportSource for RecordingSource {
    async fn team_totals(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<TeamSpendRow>, ReportError> {
        self.calls.lock().unwrap().push("teams");
        self.bounds.lock().unwrap().push((start, end));
        Ok(vec![TeamSpendRow {
            team_alias: Some("alpha".to_owned()),
            total_spend: decimal("3"),
        }])
    }

    async fn tag_totals(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<TagSpendRow>, ReportError> {
        self.calls.lock().unwrap().push("tags");
        self.bounds.lock().unwrap().push((start, end));
        Ok(vec![TagSpendRow {
            request_tag: Some("batch".to_owned()),
            total_spend: decimal("1.5"),
        }])
    }
}

struct FailingSource {
    tags_fail: bool,
    calls: Mutex<Vec<&'static str>>,
}

#[async_trait]
impl SpendReportSource for FailingSource {
    async fn team_totals(
        &self,
        _start: DateTime<Utc>,
        _end: DateTime<Utc>,
    ) -> Result<Vec<TeamSpendRow>, ReportError> {
        self.calls.lock().unwrap().push("teams");
        if !self.tags_fail {
            return Err(ReportError::Database(DbError::NoDbConnected));
        }
        Ok(Vec::new())
    }

    async fn tag_totals(
        &self,
        _start: DateTime<Utc>,
        _end: DateTime<Utc>,
    ) -> Result<Vec<TagSpendRow>, ReportError> {
        self.calls.lock().unwrap().push("tags");
        Err(ReportError::Database(DbError::NoDbConnected))
    }
}

fn midnight() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn a_missing_database_answers_none() {
    let report = get_spend_report_for_time_range(None, "2026-01-01", "2026-01-31").await;
    assert!(report.unwrap().is_none());
}

#[tokio::test]
async fn a_store_without_a_pool_answers_none() {
    let store = PostgresSpendReportStore {
        store: PostgresStore { pool: None },
    };
    let report = get_spend_report_for_time_range(Some(&store), "2026-01-01", "2026-01-31").await;
    assert!(report.unwrap().is_none());
}

#[tokio::test]
async fn a_malformed_start_date_is_an_error_before_the_query() {
    let source = RecordingSource::new();
    let error = get_spend_report_for_time_range(Some(&source), "01/01/2026", "2026-01-31")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ReportDateError::Format {
            field: "start_date",
            ..
        }
    ));
    assert!(source.calls().is_empty());
}

#[tokio::test]
async fn a_malformed_end_date_is_an_error_before_the_query() {
    let source = RecordingSource::new();
    let error = get_spend_report_for_time_range(Some(&source), "2026-01-01", "2026-13-01")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ReportDateError::Format {
            field: "end_date",
            ..
        }
    ));
    assert!(source.calls().is_empty());
}

#[tokio::test]
async fn queries_teams_then_tags_over_utc_midnights() {
    let source = RecordingSource::new();
    let (teams, tags) = get_spend_report_for_time_range(Some(&source), "2026-01-01", "2026-01-31")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.calls(), ["teams", "tags"]);
    let bounds = source.bounds.lock().unwrap().clone();
    let end = DateTime::parse_from_rfc3339("2026-01-31T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(bounds, [(midnight(), end), (midnight(), end)]);
    assert_eq!(teams[0].team_alias.as_deref(), Some("alpha"));
    assert_eq!(teams[0].total_spend, decimal("3"));
    assert_eq!(tags[0].request_tag.as_deref(), Some("batch"));
    assert_eq!(tags[0].total_spend, decimal("1.5"));
}

#[tokio::test]
async fn a_failing_team_query_answers_none_without_the_tag_query() {
    let source = FailingSource {
        tags_fail: false,
        calls: Mutex::new(Vec::new()),
    };
    let report = get_spend_report_for_time_range(Some(&source), "2026-01-01", "2026-01-31").await;
    assert!(report.unwrap().is_none());
    assert_eq!(*source.calls.lock().unwrap(), ["teams"]);
}

#[tokio::test]
async fn a_failing_tag_query_answers_none() {
    let source = FailingSource {
        tags_fail: true,
        calls: Mutex::new(Vec::new()),
    };
    let report = get_spend_report_for_time_range(Some(&source), "2026-01-01", "2026-01-31").await;
    assert!(report.unwrap().is_none());
    assert_eq!(*source.calls.lock().unwrap(), ["teams", "tags"]);
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT PRIMARY KEY,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0,
    "startTime" TIMESTAMP(3) NOT NULL,
    team_id TEXT,
    request_tags JSONB
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    team_alias TEXT
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["spend-estimates-report-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA_SQL.to_owned()])
            .resolve()
            .await
            .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

fn store(pool: &Pool) -> PostgresSpendReportStore {
    PostgresSpendReportStore {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

async fn session_timezone(pool: &Pool) -> String {
    let client = pool.get().await.unwrap();
    let row = client.query_one("SHOW TIME ZONE", &[]).await.unwrap();
    row.get(0)
}

#[tokio::test]
async fn postgres_team_totals_order_by_spend_over_the_inclusive_end_day() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias) VALUES
            ('t1', 'alpha'), ('t2', 'beta');
        INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", team_id) VALUES
            ('r1', 1, '2026-01-15 00:00:00', 't1'),
            ('r2', 2, '2026-01-15 23:59:59', 't1'),
            ('r3', 10, '2026-01-15 12:00:00', 't2'),
            ('r4', 5, '2026-01-14 23:59:59', 't1'),
            ('r5', 7, '2026-01-16 00:00:00', 't2'),
            ('r6', 0.5, '2026-01-15 06:00:00', NULL);
        "#,
    )
    .await;
    let rows = store(&pool)
        .team_totals(midnight_window(), end_window())
        .await
        .unwrap();
    let summary: Vec<(Option<&str>, rust_decimal::Decimal)> = rows
        .iter()
        .map(|row| (row.team_alias.as_deref(), row.total_spend))
        .collect();
    assert_eq!(
        summary,
        [
            (Some("beta"), decimal("10")),
            (Some("alpha"), decimal("3")),
            (None, decimal("0.5")),
        ]
    );
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_tag_totals_expand_each_request_tag() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", request_tags) VALUES
            ('r1', 1, '2026-01-15 10:00:00', '["batch","nightly"]'),
            ('r2', 2, '2026-01-15 11:00:00', '["batch"]'),
            ('r3', 3, '2026-01-15 12:00:00', NULL),
            ('r4', 9, '2026-01-16 12:00:00', '["late"]');
        "#,
    )
    .await;
    let rows = store(&pool)
        .tag_totals(midnight_window(), end_window())
        .await
        .unwrap();
    let summary: Vec<(Option<&str>, rust_decimal::Decimal)> = rows
        .iter()
        .map(|row| (row.request_tag.as_deref(), row.total_spend))
        .collect();
    assert_eq!(
        summary,
        [
            (Some("batch"), decimal("3")),
            (Some("nightly"), decimal("1")),
        ]
    );
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_report_window_ignores_the_session_timezone() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias) VALUES ('t1', 'alpha');
        INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", team_id) VALUES
            ('r1', 1, '2026-01-01 00:00:00', 't1'),
            ('r2', 2, '2026-01-31 23:59:59', 't1'),
            ('r3', 4, '2025-12-31 23:59:59', 't1'),
            ('r4', 8, '2026-02-01 00:00:00', 't1');
        "#,
    )
    .await;
    let store = store(&pool);
    execute(&pool, "SET TIME ZONE 'Pacific/Kiritimati'").await;
    assert_eq!(session_timezone(&pool).await, "Pacific/Kiritimati");
    let east = store
        .team_totals(midnight_window(), end_window())
        .await
        .unwrap();
    execute(&pool, "SET TIME ZONE 'Pacific/Pago_Pago'").await;
    assert_eq!(session_timezone(&pool).await, "Pacific/Pago_Pago");
    let west = store
        .team_totals(midnight_window(), end_window())
        .await
        .unwrap();
    assert_eq!(east, west);
    assert_eq!(east[0].total_spend, decimal("3"));
    pool.close();
    fixture.close_async().await.unwrap();
}

fn midnight_window() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn end_window() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-31T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}
