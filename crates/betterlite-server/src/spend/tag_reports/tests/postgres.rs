use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_db::database_views::definitions::DAILY_TAG_SPEND_SQL;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;

use super::super::rows::{DailyTagSpend, RawTagSpend};
use super::super::store::{PostgresTagReportStore, TagReportError, TagReportStore as _};
use super::support::decimal;

const SCHEMA_SQL: &str = r#"
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT PRIMARY KEY,
    call_type TEXT NOT NULL DEFAULT '',
    api_key TEXT NOT NULL DEFAULT '',
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    request_tags JSONB NOT NULL DEFAULT '[]'::jsonb,
    "startTime" TIMESTAMP(3) NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    "user" TEXT DEFAULT ''
);
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    key_alias TEXT,
    key_name TEXT,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    user_id TEXT,
    budget_id TEXT
);
CREATE TABLE "LiteLLM_BudgetTable" (
    budget_id TEXT PRIMARY KEY
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    user_alias TEXT,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    password TEXT,
    user_role TEXT
);
CREATE TABLE "LiteLLM_OrganizationMembership" (
    user_id TEXT,
    organization_id TEXT,
    created_at TIMESTAMP(3) NOT NULL DEFAULT '2024-01-01 00:00:00',
    updated_at TIMESTAMP(3) NOT NULL DEFAULT '2024-01-01 00:00:00'
);
"#;

const DAILY_ROWS_SQL: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, request_tags, "startTime") VALUES
    ('r1', 1.0, '["a", "b"]', '2024-01-01 10:00:00'),
    ('r2', 2.0, '["a"]', '2024-01-02 10:00:00'),
    ('r3', 4.0, '["a", "b"]', '2024-01-03 10:00:00'),
    ('r4', 100.0, '["a"]', '2024-02-01 10:00:00');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_store(pool: &Pool) -> PostgresTagReportStore {
    PostgresTagReportStore {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["tag-report-tests"])
        .url
        .map(SecretString::from);
    let statements = format!("{SCHEMA_SQL}\n{DAILY_TAG_SPEND_SQL}");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![statements])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

fn token(row: &LiteLLMVerificationToken) -> String {
    row.token.as_ref().unwrap().expose_secret().to_owned()
}

fn raw_row<'rows>(rows: &'rows [RawTagSpend], name: Option<&str>) -> &'rows RawTagSpend {
    rows.iter()
        .find(|row| row.individual_request_tag.as_deref() == name)
        .unwrap()
}

fn daily_row<'rows>(rows: &'rows [DailyTagSpend], name: Option<&str>) -> &'rows DailyTagSpend {
    rows.iter()
        .find(|row| row.individual_request_tag.as_deref() == name)
        .unwrap()
}

#[tokio::test]
async fn keys_follow_the_repository_order_and_user_scope() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_BudgetTable" (budget_id) VALUES ('budget-1');
        INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, spend, user_id, budget_id) VALUES
            ('sk-a', 'a', 1.0, 'u1', NULL),
            ('sk-b', 'b', 5.0, 'u1', 'budget-1'),
            ('sk-c', 'c', 3.0, 'u2', NULL);
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let keys = store.keys().await.unwrap();
    assert_eq!(keys.len(), 3);
    assert_eq!(
        keys.iter().map(token).collect::<Vec<_>>(),
        vec!["sk-b", "sk-c", "sk-a"]
    );
    assert!(keys[0].litellm_budget_table.is_some());
    assert!(keys[2].litellm_budget_table.is_none());
    let scoped = store.keys_for_user("u1").await.unwrap();
    assert_eq!(scoped.len(), 2);
    let mut scoped_tokens: Vec<String> = scoped.iter().map(token).collect();
    scoped_tokens.sort();
    assert_eq!(scoped_tokens, vec!["sk-a", "sk-b"]);
    assert!(store.keys_for_user("missing").await.unwrap().is_empty());
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn users_are_ranked_with_their_key_aliases() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_UserTable" (user_id, spend, password) VALUES
            ('u1', 1.0, 'hunter2'),
            ('u2', 5.0, NULL),
            ('u3', 2.0, NULL);
        INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, user_id) VALUES
            ('sk-a', 'a', 'u1'),
            ('sk-b', NULL, 'u1');
        INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id) VALUES ('u1', 'org-1');
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let users = store.users().await.unwrap();
    assert_eq!(
        users
            .iter()
            .map(|row| row.user.user_id.as_str())
            .collect::<Vec<_>>(),
        vec!["u2", "u3", "u1"]
    );
    assert_eq!(users[0].key_aliases, vec![None]);
    let mut aliases = users[2].key_aliases.clone();
    aliases.sort();
    assert_eq!(aliases, vec![None, Some("a".to_owned())]);
    let found = store.user("u1").await.unwrap().unwrap();
    assert_eq!(found.user_id, "u1");
    let memberships = found.organization_memberships.as_ref().unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].organization_id, "org-1");
    let unaffiliated = store.user("u2").await.unwrap().unwrap();
    assert_eq!(
        unaffiliated
            .organization_memberships
            .as_ref()
            .unwrap()
            .len(),
        0
    );
    assert!(store.user("missing").await.unwrap().is_none());
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn expands_repeated_tags_with_their_counts_and_spend() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, request_tags, "startTime") VALUES
            ('r1', 1.5, '["a", "b"]', '2024-01-01'),
            ('r2', 2.5, '["a"]', '2024-01-01'),
            ('r3', 9.0, '[]', '2024-01-01'),
            ('r4', 0.5, '["a", null]', '2024-01-01');
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let rows = store.raw_tag_spend().await.unwrap();
    assert_eq!(rows.len(), 3);
    let a = raw_row(&rows, Some("a"));
    assert_eq!(a.log_count, 3);
    assert_eq!(a.total_spend, decimal("4.5"));
    let b = raw_row(&rows, Some("b"));
    assert_eq!(b.log_count, 1);
    assert_eq!(b.total_spend, decimal("1.5"));
    let unknown = raw_row(&rows, None);
    assert_eq!(unknown.log_count, 1);
    assert_eq!(unknown.total_spend, decimal("0.5"));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn discovers_distinct_expanded_tag_names() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_SpendLogs" (request_id, request_tags, "startTime") VALUES
            ('r1', '["a", "b"]', '2024-01-01'),
            ('r2', '["b", "c"]', '2024-01-01'),
            ('r3', '[]', '2024-01-01'),
            ('r4', '["a", null]', '2024-01-01');
        "#,
    )
    .await;
    let store = postgres_store(&pool);
    let mut names = store.tag_names().await.unwrap().unwrap();
    names.sort();
    assert_eq!(
        names,
        vec![
            None,
            Some("a".to_owned()),
            Some("b".to_owned()),
            Some("c".to_owned())
        ]
    );
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn aggregates_the_inclusive_daily_range_by_tag() {
    let (fixture, pool) = database().await;
    execute(&pool, DAILY_ROWS_SQL).await;
    let store = postgres_store(&pool);
    let all = store
        .daily_tag_spend("2024-01-01", "2024-01-02", None)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].individual_request_tag.as_deref(), Some("a"));
    assert_eq!(all[0].total_spend, decimal("2"));
    let first = store
        .daily_tag_spend("2024-01-01", "2024-01-01", None)
        .await
        .unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(daily_row(&first, Some("a")).total_spend, decimal("1"));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn filters_the_daily_range_by_tag_list() {
    let (fixture, pool) = database().await;
    execute(&pool, DAILY_ROWS_SQL).await;
    let store = postgres_store(&pool);
    let only_a = ["a".to_owned()];
    let filtered = store
        .daily_tag_spend("2024-01-01", "2024-01-03", Some(&only_a[..]))
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].total_spend, decimal("7"));
    assert_eq!(filtered[0].log_count, 3);
    let both = ["a".to_owned(), "b".to_owned()];
    let grouped = store
        .daily_tag_spend("2024-01-01", "2024-01-03", Some(&both[..]))
        .await
        .unwrap();
    assert_eq!(grouped.len(), 2);
    assert_eq!(daily_row(&grouped, Some("b")).total_spend, decimal("5"));
    let missing = ["missing".to_owned()];
    let empty = store
        .daily_tag_spend("2024-01-01", "2024-01-03", Some(&missing[..]))
        .await
        .unwrap();
    assert!(empty.is_empty());
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn returns_every_daily_tag_past_ten() {
    let (fixture, pool) = database().await;
    let values: Vec<String> = (0_i64..12)
        .zip(1_i64..)
        .map(|(index, spend)| {
            format!("('r{index}', {spend}, '[\"tag-{index}\"]', '2024-01-01 10:00:00')")
        })
        .collect();
    let joined = values.join(", ");
    let statement = format!(
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, request_tags, "startTime") VALUES {joined}"#
    );
    execute(&pool, &statement).await;
    let store = postgres_store(&pool);
    let rows = store
        .daily_tag_spend("2024-01-01", "2024-01-01", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 12);
    assert_eq!(rows[0].total_spend, decimal("12"));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn keeps_an_unparseable_date_as_a_database_error() {
    let (fixture, pool) = database().await;
    let store = postgres_store(&pool);
    let failure = store
        .daily_tag_spend("not-a-date", "2024-01-01", None)
        .await
        .unwrap_err();
    assert!(matches!(
        failure,
        TagReportError::Database(DbError::SqlState { .. })
    ));
    let tags = ["a".to_owned()];
    let filtered = store
        .daily_tag_spend("2024-01-01", "2024-01-01", Some(&tags[..]))
        .await
        .unwrap_err();
    assert!(matches!(
        filtered,
        TagReportError::Database(DbError::SqlState { .. })
    ));
    let empty = store.daily_tag_spend("", "", None).await.unwrap_err();
    assert!(matches!(
        empty,
        TagReportError::Database(DbError::SqlState { .. })
    ));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn reports_no_database_without_a_pool() {
    let store = PostgresTagReportStore {
        store: PostgresStore { pool: None },
    };
    let missing = |result: Result<(), TagReportError>| {
        assert!(matches!(
            result,
            Err(TagReportError::Database(DbError::NoDbConnected))
        ));
    };
    missing(store.keys().await.map(drop));
    missing(store.keys_for_user("u1").await.map(drop));
    missing(store.users().await.map(drop));
    missing(store.user("u1").await.map(drop));
    missing(store.raw_tag_spend().await.map(drop));
    missing(store.tag_names().await.map(drop));
    missing(store.daily_tag_spend("s", "e", None).await.map(drop));
}
