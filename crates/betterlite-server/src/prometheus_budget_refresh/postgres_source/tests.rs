#![cfg(test)]

use std::sync::atomic::Ordering;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::user_repository::projection::UserRecordError;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{TimeZone as _, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::PostgresBudgetPageSource;
use crate::key_query_plan::response::KeyListEntry;
use crate::prometheus_budget_refresh::source::{BudgetPageError, BudgetPageSource as _};

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
        let url = Environment::parse_from(["budget-refresh-postgres-tests"])
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

    const fn source(&self) -> PostgresBudgetPageSource<'_> {
        PostgresBudgetPageSource {
            client: self.client(),
        }
    }

    async fn close(self) {
        if let Some(pool) = self.store.pool.as_ref() {
            pool.close();
        }
        self.postgres.close_async().await.unwrap();
    }
}

fn next_day() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 2, 0, 0, 0).unwrap()
}

fn optional(value: Option<&str>) -> String {
    value.map_or_else(|| String::from("NULL"), |text| format!("'{text}'"))
}

fn team_row(
    team_id: &str,
    alias: Option<&str>,
    spend: i64,
    max_budget: Option<i64>,
    created_at: &str,
) -> String {
    format!(
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, admins, members, members_with_roles, models, spend, max_budget, created_at) VALUES ('{team_id}', {}, ARRAY[]::text[], ARRAY[]::text[], '[]'::jsonb, ARRAY[]::text[], {spend}, {}, '{created_at}');"#,
        optional(alias),
        max_budget.map_or_else(|| String::from("NULL"), |value| value.to_string()),
    )
}

fn user_row(user_id: &str, spend: i64, max_budget: Option<i64>, created_at: &str) -> String {
    format!(
        r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_alias, user_email, spend, max_budget, created_at) VALUES ('{user_id}', '{user_id}-alias', '{user_id}@example.com', {spend}, {}, '{created_at}');"#,
        max_budget.map_or_else(|| String::from("NULL"), |value| value.to_string()),
    )
}

fn budget_row(budget_id: &str, max_budget: i64, reset_at: &str) -> String {
    format!(
        r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, budget_reset_at, created_by, updated_by) VALUES ('{budget_id}', {max_budget}, '{reset_at}', 'seed', 'seed');"#
    )
}

fn org_row(org_id: &str, budget_id: &str, spend: i64, created_at: &str) -> String {
    format!(
        r#"INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, spend, created_at, created_by, updated_by) VALUES ('{org_id}', '{org_id}-alias', '{budget_id}', {spend}, '{created_at}', 'seed', 'seed');"#
    )
}

fn customer_row(user_id: &str, spend: i64, budget_id: Option<&str>) -> String {
    format!(
        r#"INSERT INTO "LiteLLM_EndUserTable" (user_id, spend, budget_id) VALUES ('{user_id}', {spend}, {});"#,
        optional(budget_id),
    )
}

fn key_row(token: &str, alias: &str, spend: i64, max_budget: Option<i64>, team_id: &str) -> String {
    format!(
        r#"INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, spend, max_budget, team_id) VALUES ('{token}', '{alias}', {spend}, {}, '{team_id}');"#,
        max_budget.map_or_else(|| String::from("NULL"), |value| value.to_string()),
    )
}

#[tokio::test]
async fn teams_page_reads_alias_order_with_spend_and_reset() {
    let seed = [
        team_row(
            "t-bravo",
            Some("Bravo"),
            20,
            Some(100),
            "2026-01-02 00:00:00",
        ),
        team_row("t-alpha", Some("Alpha"), 5, None, "2026-01-03 00:00:00"),
    ]
    .concat();
    let fixture = Fixture::database(&seed).await;

    let page = fixture.source().teams_page(10, 1).await.unwrap();

    assert_eq!(page.total_count, Some(2));
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.rows[0].team_id, "t-alpha");
    assert_eq!(page.rows[0].team_alias.as_deref(), Some("Alpha"));
    assert_eq!(page.rows[0].spend, Some(Decimal::from(5_i32)));
    assert_eq!(page.rows[0].max_budget, None);
    assert_eq!(page.rows[1].team_id, "t-bravo");
    assert_eq!(page.rows[1].max_budget, Some(Decimal::from(100_i32)));
    assert_eq!(page.rows[1].budget_reset_at, None);
    fixture.close().await;
}

#[tokio::test]
async fn users_page_orders_by_created_at_descending_with_a_count_each_page() {
    let seed = [
        user_row("u-oldest", 1, None, "2026-01-01 00:00:00"),
        user_row("u-middle", 2, Some(10), "2026-01-02 00:00:00"),
        user_row("u-newest", 3, Some(20), "2026-01-03 00:00:00"),
    ]
    .concat();
    let fixture = Fixture::database(&seed).await;
    let source = fixture.source();

    let first = source.users_page(2, 1).await.unwrap();
    assert_eq!(first.total_count, Some(3));
    let ids: Vec<&str> = first.rows.iter().map(|row| row.user_id.as_str()).collect();
    assert_eq!(ids, vec!["u-newest", "u-middle"]);
    assert_eq!(first.rows[0].spend, Decimal::from(3_i32));
    assert_eq!(first.rows[0].max_budget, Some(Decimal::from(20_i32)));
    assert_eq!(
        first.rows[0].user_email.as_deref(),
        Some("u-newest@example.com")
    );

    let second = source.users_page(2, 2).await.unwrap();
    assert_eq!(second.total_count, Some(3));
    assert_eq!(second.rows.len(), 1);
    assert_eq!(second.rows[0].user_id, "u-oldest");
    fixture.close().await;
}

#[tokio::test]
async fn users_page_decodes_the_json_text_user_columns() {
    let seed = r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_alias, user_email, spend, max_budget, metadata, model_spend, model_max_budget, created_at) VALUES ('u-json', 'u-json-alias', 'u-json@example.com', 7, 50, to_jsonb('{"role": "admin"}'::text), to_jsonb('{"gpt-4": 1.25}'::text), to_jsonb('{"gpt-4": 10}'::text), '2026-01-03 00:00:00');"#;
    let fixture = Fixture::database(seed).await;
    let source = fixture.source();

    let page = source.users_page(50, 1).await.unwrap();

    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].user_id, "u-json");
    assert_eq!(page.rows[0].user_alias.as_deref(), Some("u-json-alias"));
    assert_eq!(
        page.rows[0].user_email.as_deref(),
        Some("u-json@example.com")
    );
    assert_eq!(page.rows[0].spend, Decimal::from(7_i32));
    assert_eq!(page.rows[0].max_budget, Some(Decimal::from(50_i32)));
    fixture.close().await;
}

#[tokio::test]
async fn a_malformed_json_text_user_column_is_a_user_record_error() {
    let seed = r#"INSERT INTO "LiteLLM_UserTable" (user_id, metadata, created_at) VALUES ('u-bad', to_jsonb('{'::text), '2026-01-03 00:00:00');"#;
    let fixture = Fixture::database(seed).await;
    let source = fixture.source();

    let error = source.users_page(50, 1).await.unwrap_err();

    assert!(matches!(
        error,
        BudgetPageError::UserRecord {
            source: UserRecordError::Field {
                field: "metadata",
                ..
            }
        }
    ));
    fixture.close().await;
}

#[tokio::test]
async fn orgs_page_joins_the_budget_table() {
    let seed = [
        budget_row("b-org", 50, "2030-01-02 00:00:00"),
        r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b-bare', 'seed', 'seed');"#.to_owned(),
        org_row("o-budgeted", "b-org", 10, "2026-01-01 00:00:00"),
        org_row("o-bare", "b-bare", 4, "2026-01-02 00:00:00"),
    ]
    .concat();
    let fixture = Fixture::database(&seed).await;

    let page = fixture.source().orgs_page(10, 1).await.unwrap();

    assert_eq!(page.total_count, Some(2));
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.rows[0].organization_id.as_deref(), Some("o-bare"));
    assert_eq!(page.rows[0].spend, Some(Decimal::from(4_i32)));
    assert_eq!(
        page.rows[0].budget,
        Some(BudgetTableSnapshot {
            max_budget: None,
            budget_reset_at: None,
        })
    );
    assert_eq!(page.rows[1].organization_id.as_deref(), Some("o-budgeted"));
    assert_eq!(
        page.rows[1].organization_alias.as_deref(),
        Some("o-budgeted-alias")
    );
    let budget = page.rows[1].budget.as_ref().unwrap();
    assert_eq!(budget.max_budget, Some(Decimal::from(50_i32)));
    assert_eq!(budget.budget_reset_at, Some(next_day()));
    fixture.close().await;
}

#[tokio::test]
async fn orgs_page_decodes_a_json_text_model_max_budget() {
    let seed = [
        r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, budget_reset_at, model_max_budget, created_by, updated_by) VALUES ('b-text', 30, '2030-01-02 00:00:00', to_jsonb('{"gpt-4": 10}'::text), 'seed', 'seed');"#.to_owned(),
        org_row("o-text", "b-text", 3, "2026-01-03 00:00:00"),
    ]
    .concat();
    let fixture = Fixture::database(&seed).await;

    let page = fixture.source().orgs_page(10, 1).await.unwrap();

    assert_eq!(page.rows.len(), 1);
    let budget = page.rows[0].budget.as_ref().unwrap();
    assert_eq!(budget.max_budget, Some(Decimal::from(30_i32)));
    assert_eq!(budget.budget_reset_at, Some(next_day()));
    fixture.close().await;
}

#[tokio::test]
async fn a_row_without_a_joined_budget_has_no_limits() {
    let fixture = Fixture::database("").await;
    let session = fixture.store.session().await.unwrap();
    let row = session
        .query_one("SELECT 'x'::text, NULL::text", &[])
        .await
        .unwrap();

    let budget = super::budget_of(&row).unwrap();

    assert_eq!(budget, None);
    fixture.close().await;
}

#[tokio::test]
async fn customers_page_filters_budgeted_rows_only_when_asked() {
    let seed = [
        budget_row("b-customer", 30, "2030-01-02 00:00:00"),
        customer_row("end-a", 3, Some("b-customer")),
        customer_row("end-b", 1, None),
    ]
    .concat();
    let fixture = Fixture::database(&seed).await;
    let source = fixture.source();

    let unfiltered = source.customers_page(10, 1, false).await.unwrap();
    assert_eq!(unfiltered.total_count, Some(2));
    let ids: Vec<&str> = unfiltered
        .rows
        .iter()
        .map(|row| row.user_id.as_str())
        .collect();
    assert_eq!(ids, vec!["end-a", "end-b"]);
    assert_eq!(
        unfiltered.rows[0].budget.as_ref().unwrap().max_budget,
        Some(Decimal::from(30_i32))
    );

    let filtered = source.customers_page(10, 1, true).await.unwrap();
    assert_eq!(filtered.total_count, Some(1));
    assert_eq!(filtered.rows.len(), 1);
    assert_eq!(filtered.rows[0].user_id, "end-a");
    assert_eq!(filtered.rows[0].spend, Decimal::from(3_i32));
    fixture.close().await;
}

#[tokio::test]
async fn customers_page_counts_only_the_first_page() {
    let seed = [
        customer_row("end-a", 1, None),
        customer_row("end-b", 2, None),
    ]
    .concat();
    let fixture = Fixture::database(&seed).await;
    let source = fixture.source();

    let first = source.customers_page(1, 1, false).await.unwrap();
    assert_eq!(first.total_count, Some(2));
    let second = source.customers_page(1, 2, false).await.unwrap();
    assert_eq!(second.total_count, None);
    assert_eq!(second.rows.len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn keys_page_excludes_the_dashboard_team_and_returns_full_records() {
    let seed = [
        key_row(
            "hashed-dashboard",
            "dashboard",
            1,
            None,
            "litellm-dashboard",
        ),
        key_row("hashed-real", "real", 4, Some(10), "team-a"),
    ]
    .concat();
    let fixture = Fixture::database(&seed).await;

    let page = fixture.source().keys_page(10, 1).await.unwrap();

    assert_eq!(page.total_count, Some(1));
    assert_eq!(page.rows.len(), 1);
    assert!(matches!(page.rows.first(), Some(KeyListEntry::Auth(_))));
    fixture.close().await;
}

#[tokio::test]
async fn counts_read_users_teams_and_billable_users() {
    let seed = [
        user_row("u-one", 0, None, "2026-01-01 00:00:00"),
        user_row("u-two", 0, None, "2026-01-02 00:00:00"),
        r#"INSERT INTO "LiteLLM_UserTable" (user_id, spend, metadata) VALUES ('u-inactive', 0, '{"scim_active": false}'::jsonb);"#.to_owned(),
        team_row("t-one", Some("One"), 0, None, "2026-01-01 00:00:00"),
        team_row("t-two", Some("Two"), 0, None, "2026-01-02 00:00:00"),
    ]
    .concat();
    let fixture = Fixture::database(&seed).await;
    let source = fixture.source();

    assert_eq!(source.user_count().await.unwrap(), 3);
    assert_eq!(source.billable_user_count().await.unwrap(), 2);
    assert_eq!(source.team_count().await.unwrap(), 2);
    fixture.close().await;
}

#[tokio::test]
async fn a_missing_pool_is_an_error_for_the_user_page() {
    let missing = PostgresStore { pool: None };
    let availability = Availability::default();
    let source = PostgresBudgetPageSource {
        client: PrismaClient {
            writer: &missing,
            reader: &missing,
            availability: &availability,
        },
    };

    let error = source.users_page(10, 1).await.expect_err("no pool");

    assert!(matches!(error, BudgetPageError::Database(_)));
}

#[tokio::test]
async fn teams_page_swallows_a_missing_pool() {
    let missing = PostgresStore { pool: None };
    let availability = Availability::default();
    let source = PostgresBudgetPageSource {
        client: PrismaClient {
            writer: &missing,
            reader: &missing,
            availability: &availability,
        },
    };

    let page = source.teams_page(10, 1).await.unwrap();

    assert!(page.rows.is_empty());
    assert_eq!(page.total_count, Some(0));
}

#[tokio::test]
async fn reader_fallback_serves_the_writer_once_the_replica_is_unavailable() {
    let reader = Fixture::database(&user_row("u-reader", 1, None, "2026-01-01 00:00:00")).await;
    let writer = Fixture::database(&user_row("u-writer", 2, None, "2026-01-01 00:00:00")).await;
    let source = PostgresBudgetPageSource {
        client: PrismaClient {
            writer: &writer.store,
            reader: &reader.store,
            availability: &reader.availability,
        },
    };

    let replica = source.users_page(10, 1).await.unwrap();
    assert_eq!(replica.rows[0].user_id, "u-reader");
    reader
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let primary = source.users_page(10, 1).await.unwrap();
    assert_eq!(primary.rows[0].user_id, "u-writer");
    reader.close().await;
    writer.close().await;
}
