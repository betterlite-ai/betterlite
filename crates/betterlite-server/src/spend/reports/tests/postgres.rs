use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{TimeZone as _, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::super::query_service::get_spend_by_team;
use super::super::scope::ScopeColumn;
use super::super::store::{
    OrganizationReportRequest, PostgresScopedReportStore, ScopedReportRequest,
    ScopedReportStore as _, TeamSpendRequest,
};

const SCHEMA_SQL: &str = r#"
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT PRIMARY KEY,
    api_key TEXT NOT NULL DEFAULT '',
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    "startTime" TIMESTAMP(3) NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    "user" TEXT,
    team_id TEXT,
    organization_id TEXT
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    team_alias TEXT,
    organization_id TEXT
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT NOT NULL,
    user_email TEXT,
    user_role TEXT,
    models TEXT[] DEFAULT ARRAY[]::TEXT[],
    metadata JSONB,
    spend DOUBLE PRECISION DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "LiteLLM_UserTable_pkey" PRIMARY KEY (user_id)
);
CREATE TABLE "LiteLLM_OrganizationMembership" (
    user_id TEXT NOT NULL,
    organization_id TEXT NOT NULL,
    user_role TEXT,
    spend DOUBLE PRECISION DEFAULT 0.0,
    budget_id TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "LiteLLM_OrganizationMembership_pkey" PRIMARY KEY (user_id, organization_id)
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database(rows: &str) -> (PostgresFixture, PostgresScopedReportStore) {
    let url = Environment::parse_from(["scoped-report-tests"])
        .url
        .map(SecretString::from);
    let statements = format!("{SCHEMA_SQL}\n{rows}");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![statements])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let store = PostgresScopedReportStore {
        store: PostgresStore { pool: Some(pool) },
    };
    (fixture, store)
}

fn midnight(year: i32, month: u32, day: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

const SCOPED_ROWS_SQL: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, prompt_tokens, completion_tokens, "startTime", model, "user", team_id, organization_id) VALUES
    ('before', 'k1', 100.0, 10, 6, 4, '2023-12-31 23:59:59', 'gpt', 'u1', 'team-1', 'org-1'),
    ('start', 'k1', 1.0, 3, 1, 2, '2024-01-01 00:00:00', 'gpt', 'u1', 'team-1', 'org-1'),
    ('end', 'k1', 2.0, 4, 2, 2, '2024-01-02 23:59:59', 'gpt', 'u1', 'team-1', 'org-1'),
    ('after', 'k1', 100.0, 10, 6, 4, '2024-01-03 00:00:00', 'gpt', 'u1', 'team-1', 'org-1'),
    ('k2a', 'k2', 5.0, 7, 3, 4, '2024-01-01 10:00:00', 'claude', 'u1', 'team-1', 'org-1'),
    ('k2b', 'k2', 1.0, 2, 1, 1, '2024-01-02 10:00:00', 'gpt', 'u1', 'team-1', 'org-1'),
    ('other-team', 'k3', 50.0, 5, 2, 3, '2024-01-01 12:00:00', 'gpt', 'u2', 'team-2', 'org-1');
"#;

#[tokio::test]
async fn a_scoped_report_keeps_the_start_and_end_days_only() {
    let (_fixture, store) = database(SCOPED_ROWS_SQL).await;
    let rows = store
        .scoped_report(ScopedReportRequest {
            column: ScopeColumn::ApiKey,
            start: &midnight(2024, 1, 1),
            end: &midnight(2024, 1, 2),
            value: "k1",
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].api_key, "k1");
    assert_eq!(rows[0].total_cost.to_string(), "3");
    assert_eq!(rows[0].total_input_tokens, 3);
    assert_eq!(rows[0].total_output_tokens, 4);
    assert_eq!(rows[0].model_details.len(), 1);
    assert_eq!(rows[0].model_details[0].model, "gpt");
}

#[tokio::test]
async fn a_scoped_report_orders_api_keys_by_total_cost() {
    let (_fixture, store) = database(SCOPED_ROWS_SQL).await;
    let rows = store
        .scoped_report(ScopedReportRequest {
            column: ScopeColumn::TeamId,
            start: &midnight(2024, 1, 1),
            end: &midnight(2024, 1, 2),
            value: "team-1",
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].api_key, "k2");
    assert_eq!(rows[0].total_cost.to_string(), "6");
    assert_eq!(rows[1].api_key, "k1");
    assert_eq!(rows[0].model_details.len(), 2);
    let models: Vec<&str> = rows[0]
        .model_details
        .iter()
        .map(|detail| detail.model.as_str())
        .collect();
    assert!(models.contains(&"claude"));
    assert!(models.contains(&"gpt"));
}

#[tokio::test]
async fn a_user_scope_never_sees_another_users_rows() {
    let (_fixture, store) = database(SCOPED_ROWS_SQL).await;
    let rows = store
        .scoped_report(ScopedReportRequest {
            column: ScopeColumn::User,
            start: &midnight(2024, 1, 1),
            end: &midnight(2024, 1, 2),
            value: "u1",
        })
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.api_key.as_str())
            .collect::<Vec<_>>(),
        vec!["k2", "k1"]
    );
    let other = store
        .scoped_report(ScopedReportRequest {
            column: ScopeColumn::User,
            start: &midnight(2024, 1, 1),
            end: &midnight(2024, 1, 2),
            value: "u2",
        })
        .await
        .unwrap();
    assert_eq!(other[0].api_key, "k3");
}

const USER_ROWS_SQL: &str = r#"
INSERT INTO "LiteLLM_UserTable" (user_id, user_role) VALUES ('u1', 'org_admin');
INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id, user_role) VALUES ('u1', 'org-1', 'org_admin');
"#;

#[tokio::test]
async fn find_user_reads_the_user_and_its_org_memberships() {
    let (_fixture, store) = database(USER_ROWS_SQL).await;
    let found = store.find_user("u1").await.unwrap().unwrap();
    assert_eq!(found.user_id, "u1");
    let memberships = found.organization_memberships.unwrap();
    assert_eq!(memberships[0].organization_id, "org-1");
    assert_eq!(memberships[0].user_role.as_deref(), Some("org_admin"));
    assert!(store.find_user("ghost").await.unwrap().is_none());
}

#[tokio::test]
async fn a_scoped_report_without_rows_is_empty() {
    let (_fixture, store) = database(SCOPED_ROWS_SQL).await;
    let rows = store
        .scoped_report(ScopedReportRequest {
            column: ScopeColumn::ApiKey,
            start: &midnight(2025, 1, 1),
            end: &midnight(2025, 1, 2),
            value: "k1",
        })
        .await
        .unwrap();
    assert!(rows.is_empty());
}

const ORGANIZATION_ROWS_SQL: &str = r#"
INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, organization_id) VALUES
    ('team-1', 'Alpha', 'org-1'),
    ('team-2', 'Beta', 'org-2');
INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, prompt_tokens, completion_tokens, "startTime", model, team_id, organization_id) VALUES
    ('direct', 'k4', 2.0, 3, 1, 2, '2024-01-01 10:00:00', 'gpt', NULL, 'org-1'),
    ('null-org', 'k5', 3.0, 3, 1, 2, '2024-01-01 11:00:00', 'gpt', 'team-1', NULL),
    ('empty-org', 'k6', 4.0, 3, 1, 2, '2024-01-01 12:00:00', 'gpt', 'team-1', ''),
    ('foreign-stamp', 'k7', 5.0, 3, 1, 2, '2024-01-01 13:00:00', 'gpt', 'team-1', 'org-2'),
    ('other-team', 'k8', 6.0, 3, 1, 2, '2024-01-01 14:00:00', 'gpt', 'team-2', NULL),
    ('foreign-no-team', 'k9', 7.0, 3, 1, 2, '2024-01-01 15:00:00', 'gpt', NULL, 'org-2');
"#;

#[tokio::test]
async fn an_organization_report_uses_stamps_and_team_membership_together() {
    let (_fixture, store) = database(ORGANIZATION_ROWS_SQL).await;
    let team_ids = store.organization_team_ids("org-1").await.unwrap();
    assert_eq!(team_ids, vec!["team-1"]);
    let rows = store
        .organization_report(OrganizationReportRequest {
            start: &midnight(2024, 1, 1),
            end: &midnight(2024, 1, 1),
            organization_id: "org-1",
            team_ids: &team_ids,
        })
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.api_key.as_str())
            .collect::<Vec<_>>(),
        vec!["k6", "k5", "k4"]
    );
}

#[tokio::test]
async fn an_organization_report_keeps_the_per_model_team() {
    let (_fixture, store) = database(ORGANIZATION_ROWS_SQL).await;
    let team_ids = store.organization_team_ids("org-1").await.unwrap();
    let rows = store
        .organization_report(OrganizationReportRequest {
            start: &midnight(2024, 1, 1),
            end: &midnight(2024, 1, 1),
            organization_id: "org-1",
            team_ids: &team_ids,
        })
        .await
        .unwrap();
    let unstamped = rows.iter().find(|row| row.api_key == "k5").unwrap();
    assert_eq!(
        unstamped.model_details[0].team_id.as_deref(),
        Some("team-1")
    );
    let direct = rows.iter().find(|row| row.api_key == "k4").unwrap();
    assert_eq!(direct.model_details[0].team_id, None);
}

#[tokio::test]
async fn an_organization_report_with_no_teams_keeps_only_stamped_rows() {
    let (_fixture, store) = database(ORGANIZATION_ROWS_SQL).await;
    let rows = store
        .organization_report(OrganizationReportRequest {
            start: &midnight(2024, 1, 1),
            end: &midnight(2024, 1, 1),
            organization_id: "org-1",
            team_ids: &[],
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].api_key, "k4");
}

const TEAM_DAILY_ROWS_SQL: &str = r#"
INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, organization_id) VALUES
    ('team-1', 'Alpha', 'org-1'),
    ('team-2', NULL, 'org-2');
INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, "startTime", model, team_id) VALUES
    ('d1a', 'k1', 1.0, 2, '2024-01-01 10:00:00', 'gpt', 'team-1'),
    ('d1b', 'k1', 3.0, 4, '2024-01-01 11:00:00', 'claude', 'team-1'),
    ('d1c', 'k2', 2.0, 6, '2024-01-01 12:00:00', 'gpt', 'team-2'),
    ('d2a', 'k1', 5.0, 8, '2024-01-02 10:00:00', 'gpt', 'team-1');
"#;

#[tokio::test]
async fn spend_by_team_groups_days_teams_models_and_keys() {
    let (_fixture, store) = database(TEAM_DAILY_ROWS_SQL).await;
    let rows = get_spend_by_team(
        &store.store,
        &midnight(2024, 1, 1),
        &midnight(2024, 1, 2),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.group_by_day.as_str())
            .collect::<Vec<_>>(),
        vec!["2024-01-01T00:00:00.000Z", "2024-01-02T00:00:00.000Z"]
    );
    let first = &rows[0];
    let alpha = first
        .teams
        .iter()
        .find(|team| team.team_name == "Alpha")
        .unwrap();
    assert_eq!(alpha.total_spend.to_string(), "4");
    assert_eq!(alpha.metadata.len(), 2);
    let gpt = alpha
        .metadata
        .iter()
        .find(|entry| entry.model == "gpt")
        .unwrap();
    assert_eq!(gpt.api_key, "k1");
    assert_eq!(gpt.spend.to_string(), "1");
    assert_eq!(gpt.total_tokens, 2);
    let unassigned = first
        .teams
        .iter()
        .find(|team| team.team_name == "Unassigned Team")
        .unwrap();
    assert_eq!(unassigned.total_spend.to_string(), "2");
}

#[tokio::test]
async fn spend_by_team_applies_the_optional_team_filter() {
    let (_fixture, store) = database(TEAM_DAILY_ROWS_SQL).await;
    let rows = get_spend_by_team(
        &store.store,
        &midnight(2024, 1, 1),
        &midnight(2024, 1, 2),
        Some("team-1"),
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| { row.teams.iter().all(|team| team.team_name == "Alpha") })
    );
    assert_eq!(rows[0].teams.len(), 1);
    assert_eq!(rows[1].teams[0].total_spend.to_string(), "5");
}

#[tokio::test]
async fn spend_by_team_through_the_store_matches_the_query_service() {
    let (_fixture, store) = database(TEAM_DAILY_ROWS_SQL).await;
    let rows = store
        .spend_by_team(TeamSpendRequest {
            start: &midnight(2024, 1, 1),
            end: &midnight(2024, 1, 1),
            team_id: Some("team-2"),
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].teams[0].team_name, "Unassigned Team");
    assert_eq!(rows[0].teams[0].metadata[0].api_key, "k2");
}
