use std::collections::BTreeSet;

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;
use sha2::{Digest as _, Sha256};
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::JsonData;
use crate::finops::focus::{FocusUsageError, UsageRow, get_usage_data, usage_query};
use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;

const MIGRATIONS: &str = concat!(
    include_str!("../../../../migrations/20250326162113_baseline/migration.sql"),
    include_str!("../../../../migrations/20250326171002_add_daily_user_table/migration.sql"),
    include_str!(
        "../../../../migrations/20250327180120_add_api_requests_to_daily_user_table/migration.sql"
    ),
    include_str!(
        "../../../../migrations/20250331215456_track_success_and_failed_requests_daily_agg_table/migration.sql"
    ),
    include_str!(
        "../../../../migrations/20250415151647_add_cache_read_write_tokens_daily_spend_transactions/migration.sql"
    ),
    include_str!("../../../../migrations/20250415191926_add_daily_team_table/migration.sql"),
    include_str!("../../../../migrations/20250416115320_add_tag_table_to_db/migration.sql"),
    include_str!(
        "../../../../migrations/20250509141545_use_big_int_for_daily_spend_tables/migration.sql"
    ),
    include_str!("../../../../migrations/20250526154401_allow_null_entity_id/migration.sql"),
    include_str!(
        "../../../../migrations/20250707230009_add_mcp_namespaced_tool_name/migration.sql"
    ),
    include_str!(
        "../../../../migrations/20251219110931_add_deleted_keys_and_deleted_teams_tables/migration.sql"
    ),
);

const EXPECTED_SQL: &str = r#"SELECT dus.id, dus.date, dus.user_id, dus.api_key, dus.model,
    dus.model_group, dus.custom_llm_provider, dus.prompt_tokens, dus.completion_tokens,
    dus.spend::numeric AS spend, dus.api_requests, dus.successful_requests,
    dus.failed_requests, dus.cache_creation_input_tokens, dus.cache_read_input_tokens,
    dus.created_at, dus.updated_at, vt.team_id, vt.key_alias AS api_key_alias, tt.team_alias,
    ut.user_email AS user_email,
    COALESCE(vt.organization_id, tt.organization_id) AS organization_id,
    organization.organization_alias AS organization_alias FROM "LiteLLM_DailyUserSpend" dus
    LEFT JOIN "LiteLLM_VerificationToken" vt ON dus.api_key = vt.token
    LEFT JOIN "LiteLLM_TeamTable" tt ON vt.team_id = tt.team_id
    LEFT JOIN "LiteLLM_UserTable" ut ON dus.user_id = ut.user_id
    LEFT JOIN "LiteLLM_OrganizationTable" organization
    ON organization.organization_id = COALESCE(vt.organization_id, tt.organization_id)
    ORDER BY dus.date DESC, dus.created_at DESC"#;

const FOCUS_COLUMNS: [&str; 23] = [
    "id",
    "date",
    "user_id",
    "api_key",
    "model",
    "model_group",
    "custom_llm_provider",
    "prompt_tokens",
    "completion_tokens",
    "spend",
    "api_requests",
    "successful_requests",
    "failed_requests",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "created_at",
    "updated_at",
    "team_id",
    "api_key_alias",
    "team_alias",
    "user_email",
    "organization_id",
    "organization_alias",
];

const USAGE_INSERT: &str = r#"INSERT INTO "LiteLLM_DailyUserSpend"
    (id, user_id, date, api_key, model, spend, created_at, updated_at)
    VALUES ($1, 'user-1', $2, $3, 'gpt-4o', 1.5, $4, $5)"#;
const USAGE_NULL_MODEL_INSERT: &str = r#"INSERT INTO "LiteLLM_DailyUserSpend"
    (id, user_id, date, api_key, model, spend, created_at, updated_at)
    VALUES ($1, 'user-1', $2, $3, NULL, 1.5, $4, $5)"#;
const BUDGET_INSERT: &str = r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by)
    VALUES ($1, 'test', 'test')"#;
const ORGANIZATION_INSERT: &str = r#"INSERT INTO "LiteLLM_OrganizationTable"
    (organization_id, organization_alias, budget_id, created_by, updated_by)
    VALUES ($1, $2, $3, 'test', 'test')"#;
const TEAM_INSERT: &str = r#"INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, organization_id)
    VALUES ($1, $2, $3)"#;
const USER_INSERT: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ($1, $2)"#;
const TOKEN_INSERT: &str = r#"INSERT INTO "LiteLLM_VerificationToken"
    (token, key_alias, user_id, team_id, organization_id)
    VALUES ($1, $2, $3, $4, $5)"#;

fn normalize(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn moment(day: u32, hour: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 6, day)
        .unwrap()
        .and_hms_opt(hour, 0, 0)
        .unwrap()
}

struct UsageFixture<'row> {
    id: &'row str,
    api_key: &'row str,
    date: &'row str,
    at: NaiveDateTime,
}

impl UsageFixture<'_> {
    fn values(&self) -> [&(dyn ToSql + Sync); 5] {
        [&self.id, &self.date, &self.api_key, &self.at, &self.at]
    }
}

fn usage_row<'row>(
    id: &'row str,
    api_key: &'row str,
    date: &'row str,
    at: NaiveDateTime,
) -> UsageFixture<'row> {
    UsageFixture {
        id,
        api_key,
        date,
        at,
    }
}

struct TokenFixture<'row> {
    token: &'row str,
    alias: &'row str,
    user_id: Option<&'row str>,
    team_id: Option<&'row str>,
    organization_id: Option<&'row str>,
}

impl TokenFixture<'_> {
    fn values(&self) -> [&(dyn ToSql + Sync); 5] {
        [
            &self.token,
            &self.alias,
            &self.user_id,
            &self.team_id,
            &self.organization_id,
        ]
    }
}

fn token<'row>(
    text: &'row str,
    alias: &'row str,
    team_id: &'row str,
    organization_id: Option<&'row str>,
) -> TokenFixture<'row> {
    TokenFixture {
        token: text,
        alias,
        user_id: Some("user-1"),
        team_id: Some(team_id),
        organization_id,
    }
}

fn spend(value: Decimal) -> JsonData {
    serde_json::from_str(&value.to_string()).unwrap()
}

fn text_of<'row>(row: &'row UsageRow, field: &str) -> Option<&'row str> {
    match row.get(field) {
        Some(JsonData::String(text)) => Some(text.as_str()),
        Some(_) | None => None,
    }
}

fn ids(usage: &[UsageRow]) -> Vec<&str> {
    usage.iter().filter_map(|row| text_of(row, "id")).collect()
}

fn by_id<'usage>(usage: &'usage [UsageRow], id: &str) -> &'usage UsageRow {
    usage
        .iter()
        .find(|row| text_of(row, "id") == Some(id))
        .unwrap()
}

async fn insert(store: &PostgresStore, sql: &str, params: &[&(dyn ToSql + Sync)]) {
    store
        .pool
        .as_ref()
        .unwrap()
        .get()
        .await
        .unwrap()
        .execute(sql, params)
        .await
        .unwrap();
}

async fn insert_usage(store: &PostgresStore, row: &UsageFixture<'_>) {
    insert(store, USAGE_INSERT, &row.values()).await;
}

async fn insert_usage_without_model(store: &PostgresStore, row: &UsageFixture<'_>) {
    insert(store, USAGE_NULL_MODEL_INSERT, &row.values()).await;
}

async fn insert_usage_rows(store: &PostgresStore, rows: &[UsageFixture<'_>]) {
    for row in rows {
        insert_usage(store, row).await;
    }
}

async fn insert_organization(store: &PostgresStore, organization_id: &str, alias: &str) {
    let budget = format!("budget-{organization_id}");
    insert(store, BUDGET_INSERT, &[&budget]).await;
    insert(
        store,
        ORGANIZATION_INSERT,
        &[&organization_id, &alias, &budget],
    )
    .await;
}

async fn insert_team(
    store: &PostgresStore,
    team_id: &str,
    alias: &str,
    organization_id: Option<&str>,
) {
    insert(store, TEAM_INSERT, &[&team_id, &alias, &organization_id]).await;
}

async fn insert_user(store: &PostgresStore, user_id: &str, email: &str) {
    insert(store, USER_INSERT, &[&user_id, &email]).await;
}

async fn insert_token(store: &PostgresStore, token: &TokenFixture<'_>) {
    insert(store, TOKEN_INSERT, &token.values()).await;
}

async fn insert_full_graph(store: &PostgresStore) {
    insert_organization(store, "org-1", "Org One").await;
    insert_team(store, "team-1", "Team One", Some("org-1")).await;
    insert_user(store, "user-1", "user@example.com").await;
    insert_token(
        store,
        &token("sk-token", "alias-1", "team-1", Some("org-1")),
    )
    .await;
}

async fn database() -> (PostgresFixture, PostgresStore) {
    use clap::Parser as _;
    use secrecy::SecretString;

    #[derive(clap::Parser)]
    struct Environment {
        #[arg(long, env = "POSTGRES_TEST_URL")]
        url: Option<String>,
    }

    let url = Environment::parse_from(["finops-focus-usage-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let mut config = fixture.options.config.clone();
    let current = config.get_options().unwrap_or_default().to_owned();
    config.options(format!("{current} -ctimezone=UTC"));
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(3)
        .build()
        .unwrap();
    pool.get()
        .await
        .unwrap()
        .batch_execute(MIGRATIONS)
        .await
        .unwrap();
    (fixture, PostgresStore { pool: Some(pool) })
}

async fn close(fixture: PostgresFixture, store: PostgresStore) {
    if let Some(pool) = store.pool {
        pool.close();
    }
    fixture.close_async().await.unwrap();
}

#[test]
fn focus_selects_every_litellm_column() {
    let query = usage_query(None, None, None).unwrap();
    assert_eq!(normalize(&query.sql), normalize(EXPECTED_SQL));
    assert_eq!(query.limit, None);
    assert_eq!(query.start_time_utc, None);
    assert_eq!(query.end_time_utc, None);
}

#[test]
fn join_coverage_reaches_the_four_joined_tables() {
    let sql = normalize(&usage_query(None, None, None).unwrap().sql);
    let verification = r#"LEFT JOIN "LiteLLM_VerificationToken" vt ON dus.api_key = vt.token"#;
    let team = r#"LEFT JOIN "LiteLLM_TeamTable" tt ON vt.team_id = tt.team_id"#;
    let user = r#"LEFT JOIN "LiteLLM_UserTable" ut ON dus.user_id = ut.user_id"#;
    let organization = r#"LEFT JOIN "LiteLLM_OrganizationTable" organization ON organization.organization_id = COALESCE(vt.organization_id, tt.organization_id)"#;
    assert!(sql.contains(verification), "missing the token join");
    assert!(sql.contains(team), "missing the team join");
    assert!(sql.contains(user), "missing the user join");
    assert!(sql.contains(organization), "missing the organization join");
}

#[rstest]
#[case::start_only(
    Some(moment(15, 12).and_utc()),
    None,
    "dus.updated_at >= $1::timestamptz",
    1
)]
#[case::end_only(
    None,
    Some(moment(17, 12).and_utc()),
    "dus.updated_at <= $1::timestamptz",
    1
)]
#[case::both(
    Some(moment(15, 12).and_utc()),
    Some(moment(17, 12).and_utc()),
    "dus.updated_at >= $1::timestamptz AND dus.updated_at <= $2::timestamptz",
    2
)]
fn bounds_take_only_the_present_sides(
    #[case] start: Option<DateTime<Utc>>,
    #[case] end: Option<DateTime<Utc>>,
    #[case] clause: &str,
    #[case] bound: usize,
) {
    let query = usage_query(None, start, end).unwrap();
    let sql = normalize(&query.sql);
    assert!(sql.contains(clause), "missing {clause}");
    assert!(sql.ends_with("ORDER BY dus.date DESC, dus.created_at DESC"));
    assert_eq!(query.bind().len(), bound);
    assert_eq!(query.start_time_utc, start);
    assert_eq!(query.end_time_utc, end);
}

#[test]
fn bounds_skip_the_filter_without_bounds() {
    let query = usage_query(None, None, None).unwrap();
    assert!(!normalize(&query.sql).contains("WHERE"));
    assert_eq!(query.bind().len(), 0);
}

#[tokio::test]
async fn bounds_include_rows_on_both_boundaries() {
    let (fixture, store) = database().await;
    let rows = [
        usage_row("before", "sk-before", "2026-06-13", moment(14, 12)),
        usage_row("at-start", "sk-start", "2026-06-14", moment(15, 12)),
        usage_row("middle", "sk-middle", "2026-06-15", moment(16, 0)),
        usage_row("at-end", "sk-end", "2026-06-16", moment(17, 12)),
        usage_row("after", "sk-after", "2026-06-17", moment(18, 0)),
    ];
    insert_usage_rows(&store, &rows).await;
    let start = moment(15, 12).and_utc();
    let end = moment(17, 12).and_utc();
    let usage = get_usage_data(&store, None, Some(start), Some(end))
        .await
        .unwrap();
    assert_eq!(ids(&usage), ["at-end", "middle", "at-start"]);
    close(fixture, store).await;
}

#[tokio::test]
async fn ordering_reads_newest_date_then_newest_creation() {
    let (fixture, store) = database().await;
    let rows = [
        usage_row("oldest-date", "sk-oldest", "2026-06-14", moment(14, 9)),
        usage_row("older-creation", "sk-older", "2026-06-15", moment(15, 8)),
        usage_row("newer-creation", "sk-newer", "2026-06-15", moment(15, 10)),
        usage_row("newest-date", "sk-newest", "2026-06-16", moment(16, 7)),
    ];
    insert_usage_rows(&store, &rows).await;
    let usage = get_usage_data(&store, None, None, None).await.unwrap();
    let expected = [
        "newest-date",
        "newer-creation",
        "older-creation",
        "oldest-date",
    ];
    assert_eq!(ids(&usage), expected);
    close(fixture, store).await;
}

#[tokio::test]
async fn focus_limit_rejects_negative_values() {
    assert!(matches!(
        usage_query(Some(-1), None, None),
        Err(FocusUsageError::NegativeLimit)
    ));
    assert_eq!(
        FocusUsageError::NegativeLimit.to_string(),
        "limit must be non-negative"
    );
    let store = PostgresStore { pool: None };
    let usage = get_usage_data(&store, Some(-2), None, None).await;
    assert!(matches!(usage, Err(FocusUsageError::NegativeLimit)));
}

#[rstest]
#[case::zero(0_i64)]
#[case::one(1_i64)]
#[case::fifty(50_i64)]
fn focus_limit_accepts_zero_and_positive_values(#[case] limit: i64) {
    let query = usage_query(Some(limit), None, None).unwrap();
    assert_eq!(query.limit, Some(limit));
    assert!(query.sql.ends_with("LIMIT $1"));
    assert_eq!(query.bind().len(), 1);
}

#[rstest]
#[case::only_limit(None, None, "LIMIT $1", 1)]
#[case::start_and_limit(Some(moment(15, 12).and_utc()), None, "LIMIT $2", 2)]
#[case::end_and_limit(None, Some(moment(17, 12).and_utc()), "LIMIT $2", 2)]
#[case::both_and_limit(
    Some(moment(15, 12).and_utc()),
    Some(moment(17, 12).and_utc()),
    "LIMIT $3",
    3
)]
fn focus_limit_follows_the_bound_placeholders(
    #[case] start: Option<DateTime<Utc>>,
    #[case] end: Option<DateTime<Utc>>,
    #[case] clause: &str,
    #[case] bound: usize,
) {
    let query = usage_query(Some(7), start, end).unwrap();
    assert!(query.sql.ends_with(clause), "missing {clause}");
    assert_eq!(query.bind().len(), bound);
}

#[tokio::test]
async fn focus_limit_returns_at_most_the_limit_rows() {
    let (fixture, store) = database().await;
    let rows = [
        usage_row("one", "sk-one", "2026-06-16", moment(16, 12)),
        usage_row("two", "sk-two", "2026-06-15", moment(15, 12)),
        usage_row("three", "sk-three", "2026-06-14", moment(14, 12)),
    ];
    insert_usage_rows(&store, &rows).await;
    let limited = get_usage_data(&store, Some(2), None, None).await.unwrap();
    assert_eq!(ids(&limited), ["one", "two"]);
    let empty = get_usage_data(&store, Some(0), None, None).await.unwrap();
    assert!(empty.is_empty());
    let all = get_usage_data(&store, None, None, None).await.unwrap();
    assert_eq!(all.len(), 3);
    close(fixture, store).await;
}

#[tokio::test]
async fn dataframe_rows_hold_every_selected_column() {
    let (fixture, store) = database().await;
    insert_full_graph(&store).await;
    let row = usage_row("row-1", "sk-token", "2026-06-15", moment(15, 12));
    insert_usage(&store, &row).await;
    let usage = get_usage_data(&store, None, None, None).await.unwrap();
    assert_eq!(usage.len(), 1);
    let loaded = &usage[0];
    let columns: BTreeSet<&str> = loaded.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = BTreeSet::from(FOCUS_COLUMNS);
    assert_eq!(columns, expected);
    assert_eq!(text_of(loaded, "api_key_alias"), Some("alias-1"));
    assert_eq!(text_of(loaded, "organization_alias"), Some("Org One"));
    close(fixture, store).await;
}

#[tokio::test]
async fn dataframe_rows_keep_money_counters_and_timestamps() {
    let (fixture, store) = database().await;
    let row = usage_row("row-1", "sk-token", "2026-06-15", moment(15, 12));
    insert_usage(&store, &row).await;
    let usage = get_usage_data(&store, None, None, None).await.unwrap();
    let loaded = &usage[0];
    assert_eq!(loaded.get("spend"), Some(&spend(Decimal::new(15, 1))));
    assert_eq!(
        loaded.get("prompt_tokens"),
        Some(&JsonData::Number(Number::from(0_i64)))
    );
    assert_eq!(
        loaded.get("api_requests"),
        Some(&JsonData::Number(Number::from(0_i64)))
    );
    assert_eq!(text_of(loaded, "created_at"), Some("2026-06-15T12:00:00Z"));
    assert_eq!(text_of(loaded, "updated_at"), Some("2026-06-15T12:00:00Z"));
    close(fixture, store).await;
}

#[tokio::test]
async fn dataframe_rows_keep_null_model_as_null() {
    let (fixture, store) = database().await;
    let rows = [
        usage_row("with-model", "sk-with-model", "2026-06-16", moment(16, 12)),
        usage_row("null-model", "sk-null-model", "2026-06-15", moment(15, 12)),
    ];
    insert_usage(&store, &rows[0]).await;
    insert_usage_without_model(&store, &rows[1]).await;
    let usage = get_usage_data(&store, None, None, None).await.unwrap();
    assert_eq!(ids(&usage), ["with-model", "null-model"]);
    let null_model = by_id(&usage, "null-model");
    assert_eq!(null_model.get("model"), Some(&JsonData::Null));
    let with_model = by_id(&usage, "with-model");
    assert_eq!(text_of(with_model, "model"), Some("gpt-4o"));
    close(fixture, store).await;
}

#[tokio::test]
async fn dataframe_is_empty_without_matching_rows() {
    let (fixture, store) = database().await;
    assert!(
        get_usage_data(&store, None, None, None)
            .await
            .unwrap()
            .is_empty()
    );
    let row = usage_row("only", "sk-only", "2026-06-15", moment(15, 12));
    insert_usage(&store, &row).await;
    let end = moment(14, 12).and_utc();
    let filtered = get_usage_data(&store, None, None, Some(end)).await.unwrap();
    assert!(filtered.is_empty());
    close(fixture, store).await;
}

#[tokio::test]
async fn join_coverage_resolves_the_joined_fields() {
    let (fixture, store) = database().await;
    insert_organization(&store, "org-1", "Org One").await;
    insert_organization(&store, "org-2", "Org Two").await;
    insert_team(&store, "team-1", "Team One", Some("org-2")).await;
    insert_team(&store, "team-2", "Team Two", Some("org-2")).await;
    insert_user(&store, "user-1", "user@example.com").await;
    insert_token(
        &store,
        &token("sk-with-org", "alias-one", "team-1", Some("org-1")),
    )
    .await;
    insert_token(&store, &token("sk-team-org", "alias-two", "team-2", None)).await;
    let rows = [
        usage_row("with-org", "sk-with-org", "2026-06-15", moment(15, 12)),
        usage_row("team-org", "sk-team-org", "2026-06-14", moment(14, 12)),
    ];
    insert_usage_rows(&store, &rows).await;
    let usage = get_usage_data(&store, None, None, None).await.unwrap();
    let with_org = by_id(&usage, "with-org");
    assert_eq!(text_of(with_org, "api_key_alias"), Some("alias-one"));
    assert_eq!(text_of(with_org, "team_alias"), Some("Team One"));
    assert_eq!(text_of(with_org, "organization_id"), Some("org-1"));
    assert_eq!(text_of(with_org, "organization_alias"), Some("Org One"));
    let team_org = by_id(&usage, "team-org");
    assert_eq!(text_of(team_org, "organization_id"), Some("org-2"));
    assert_eq!(text_of(team_org, "organization_alias"), Some("Org Two"));
    close(fixture, store).await;
}

#[tokio::test]
async fn join_coverage_leaves_unmatched_rows_null() {
    let (fixture, store) = database().await;
    let row = usage_row("unmatched", "sk-unmatched", "2026-06-15", moment(15, 12));
    insert_usage(&store, &row).await;
    let usage = get_usage_data(&store, None, None, None).await.unwrap();
    let loaded = &usage[0];
    assert_eq!(loaded.get("team_id"), Some(&JsonData::Null));
    assert_eq!(loaded.get("api_key_alias"), Some(&JsonData::Null));
    assert_eq!(loaded.get("team_alias"), Some(&JsonData::Null));
    assert_eq!(loaded.get("user_email"), Some(&JsonData::Null));
    assert_eq!(loaded.get("organization_id"), Some(&JsonData::Null));
    assert_eq!(loaded.get("organization_alias"), Some(&JsonData::Null));
    close(fixture, store).await;
}

#[tokio::test]
async fn alias_recovery_fills_aliases_for_hashed_keys() {
    let (fixture, store) = database().await;
    insert_user(&store, "user-9", "recovered@example.com").await;
    insert_token(
        &store,
        &TokenFixture {
            token: "sk-original",
            alias: "recovered-alias",
            user_id: Some("user-9"),
            team_id: Some("team-9"),
            organization_id: None,
        },
    )
    .await;
    let hashed = hex::encode(Sha256::digest(b"sk-original"));
    let row = usage_row("hashed", &hashed, "2026-06-15", moment(15, 12));
    insert_usage(&store, &row).await;
    let usage = get_usage_data(&store, None, None, None).await.unwrap();
    let loaded = &usage[0];
    assert_eq!(text_of(loaded, "api_key_alias"), Some("recovered-alias"));
    assert_eq!(text_of(loaded, "team_id"), Some("team-9"));
    assert_eq!(text_of(loaded, "user_email"), Some("recovered@example.com"));
    close(fixture, store).await;
}

#[tokio::test]
async fn alias_recovery_keeps_unknown_keys_null() {
    let (fixture, store) = database().await;
    let row = usage_row("unknown", "sk-unknown", "2026-06-15", moment(15, 12));
    insert_usage(&store, &row).await;
    let usage = get_usage_data(&store, None, None, None).await.unwrap();
    assert_eq!(usage[0].get("api_key_alias"), Some(&JsonData::Null));
    close(fixture, store).await;
}

#[tokio::test]
async fn focus_database_is_required() {
    let store = PostgresStore { pool: None };
    let result = get_usage_data(&store, None, None, None).await;
    assert!(matches!(
        result,
        Err(FocusUsageError::Database {
            source: DbError::NoDbConnected
        })
    ));
}
