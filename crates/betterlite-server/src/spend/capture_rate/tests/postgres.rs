use std::collections::BTreeMap;

use chrono::NaiveDate;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;

use super::super::store::{
    CapturedSpendError, CapturedSpendRequest, CapturedSpendSource as _, PostgresCapturedSpend,
};

const SCHEMA_SQL: &str = r#"
CREATE TABLE "LiteLLM_DailyUserSpend" (
    date TEXT NOT NULL,
    custom_llm_provider TEXT,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0
);
"#;

const PROVIDER_ROWS_SQL: &str = r#"
INSERT INTO "LiteLLM_DailyUserSpend" (date, custom_llm_provider, spend) VALUES
    ('2026-11-14', 'openai', 9.0),
    ('2026-11-15', 'openai', 1.5),
    ('2026-11-15', 'text-completion-openai', 2.25),
    ('2026-11-15', 'azure', 100.0),
    ('2026-11-17', 'openai', 0.25);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn date(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

fn captured_spend(pool: &Pool) -> PostgresCapturedSpend {
    PostgresCapturedSpend {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn captured_days(
    source: &PostgresCapturedSpend,
    providers: &[&str],
    start: &str,
    end: &str,
) -> BTreeMap<String, Decimal> {
    source
        .captured_by_day(&CapturedSpendRequest {
            providers,
            start_date: date(start),
            end_date: date(end),
        })
        .await
        .unwrap()
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["capture-rate-tests"])
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

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

#[tokio::test]
async fn sums_provider_rows_over_inclusive_dates() {
    let (fixture, pool) = database().await;
    execute(&pool, PROVIDER_ROWS_SQL).await;
    let source = captured_spend(&pool);
    let providers = ["openai", "text-completion-openai"];
    let days = captured_days(&source, &providers, "2026-11-15", "2026-11-17").await;
    assert_eq!(
        days,
        BTreeMap::from([
            ("2026-11-15".to_owned(), dec("3.75")),
            ("2026-11-17".to_owned(), dec("0.25")),
        ])
    );
    let one_day = captured_days(&source, &providers, "2026-11-15", "2026-11-15").await;
    assert_eq!(
        one_day,
        BTreeMap::from([("2026-11-15".to_owned(), dec("3.75"))])
    );
    let duplicated =
        captured_days(&source, &["openai", "openai"], "2026-11-15", "2026-11-15").await;
    assert_eq!(
        duplicated,
        BTreeMap::from([("2026-11-15".to_owned(), dec("1.5"))])
    );
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn an_empty_window_has_no_days() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (date, custom_llm_provider, spend) VALUES
            ('2026-11-14', 'openai', 9.0);
        "#,
    )
    .await;
    let source = captured_spend(&pool);
    let days = source
        .captured_by_day(&CapturedSpendRequest {
            providers: &["openai"],
            start_date: date("2026-11-15"),
            end_date: date("2026-11-17"),
        })
        .await
        .unwrap();
    assert!(days.is_empty());
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn reports_an_unreadable_captured_total() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        r#"
        INSERT INTO "LiteLLM_DailyUserSpend" (date, custom_llm_provider, spend) VALUES
            ('2026-11-15', 'openai', 'NaN'::double precision);
        "#,
    )
    .await;
    let source = captured_spend(&pool);
    let error = source
        .captured_by_day(&CapturedSpendRequest {
            providers: &["openai"],
            start_date: date("2026-11-15"),
            end_date: date("2026-11-15"),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CapturedSpendError::UnreadableAmount { date, .. } if date == "2026-11-15"
    ));
    pool.close();
    fixture.close_async().await.unwrap();
}
