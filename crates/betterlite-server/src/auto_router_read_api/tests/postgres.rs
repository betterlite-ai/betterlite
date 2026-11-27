use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDate;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use super::query;
use crate::auto_router_read_api::postgres::PgAutoRouterReads;
use crate::auto_router_read_api::service::benchmarks_for;
use crate::auto_router_read_api::store::AutoRouterReadStore;
use crate::benchmark_aggregates::window::BenchmarkWindow;

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_AutoRouterSession" (
    "api_key" TEXT NOT NULL,
    "session_id" TEXT NOT NULL,
    "router_name" TEXT NOT NULL,
    "router_type" TEXT NOT NULL,
    "first_turn_at" TIMESTAMP(3) NOT NULL,
    "last_turn_at" TIMESTAMP(3) NOT NULL,
    "last_model" TEXT NOT NULL,
    "models" JSONB NOT NULL DEFAULT '{}',
    "turns" INTEGER NOT NULL DEFAULT 0,
    "unordered_turns" INTEGER NOT NULL DEFAULT 0,
    "covered_turns" INTEGER NOT NULL DEFAULT 0,
    "cache_hits" INTEGER NOT NULL DEFAULT 0,
    "same_model_turns" INTEGER NOT NULL DEFAULT 0,
    "same_model_hits" INTEGER NOT NULL DEFAULT 0,
    "first_visit_turns" INTEGER NOT NULL DEFAULT 0,
    "first_visit_hits" INTEGER NOT NULL DEFAULT 0,
    "return_turns" INTEGER NOT NULL DEFAULT 0,
    "return_hits" INTEGER NOT NULL DEFAULT 0,
    "return_expired_misses" INTEGER NOT NULL DEFAULT 0,
    "return_within_ttl_misses" INTEGER NOT NULL DEFAULT 0,
    "ttl_5m_turns" INTEGER NOT NULL DEFAULT 0,
    "ttl_1h_turns" INTEGER NOT NULL DEFAULT 0,
    "total_tokens" BIGINT NOT NULL DEFAULT 0,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "saved_spend" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "savings_estimated_turns" INTEGER NOT NULL DEFAULT 0,
    "savings_estimated_actual_spend" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "savings_estimated_saved_spend" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "savings_estimated_baseline_models" JSONB NOT NULL DEFAULT '{}',
    "classifier_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "classifier_cost_recorded_turns" INTEGER NOT NULL DEFAULT 0,
    "tier_turns" JSONB NOT NULL DEFAULT '{}',
    "baseline_models" JSONB NOT NULL DEFAULT '{}',
    CONSTRAINT "LiteLLM_AutoRouterSession_pkey" PRIMARY KEY ("api_key", "session_id", "router_name")
);
CREATE TABLE "LiteLLM_AutoRouterUserSession" (
    "user_id" TEXT NOT NULL,
    "api_key" TEXT NOT NULL,
    "session_id" TEXT NOT NULL,
    "router_name" TEXT NOT NULL,
    "router_type" TEXT NOT NULL,
    "first_turn_at" TIMESTAMP(3) NOT NULL,
    "last_turn_at" TIMESTAMP(3) NOT NULL,
    "last_model" TEXT NOT NULL,
    "models" JSONB NOT NULL DEFAULT '{}',
    "turns" INTEGER NOT NULL DEFAULT 0,
    "unordered_turns" INTEGER NOT NULL DEFAULT 0,
    "covered_turns" INTEGER NOT NULL DEFAULT 0,
    "cache_hits" INTEGER NOT NULL DEFAULT 0,
    "same_model_turns" INTEGER NOT NULL DEFAULT 0,
    "same_model_hits" INTEGER NOT NULL DEFAULT 0,
    "first_visit_turns" INTEGER NOT NULL DEFAULT 0,
    "first_visit_hits" INTEGER NOT NULL DEFAULT 0,
    "return_turns" INTEGER NOT NULL DEFAULT 0,
    "return_hits" INTEGER NOT NULL DEFAULT 0,
    "return_expired_misses" INTEGER NOT NULL DEFAULT 0,
    "return_within_ttl_misses" INTEGER NOT NULL DEFAULT 0,
    "ttl_5m_turns" INTEGER NOT NULL DEFAULT 0,
    "ttl_1h_turns" INTEGER NOT NULL DEFAULT 0,
    "total_tokens" BIGINT NOT NULL DEFAULT 0,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "saved_spend" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "savings_estimated_turns" INTEGER NOT NULL DEFAULT 0,
    "savings_estimated_actual_spend" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "savings_estimated_saved_spend" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "savings_estimated_baseline_models" JSONB NOT NULL DEFAULT '{}',
    "classifier_cost" DOUBLE PRECISION NOT NULL DEFAULT 0,
    "classifier_cost_recorded_turns" INTEGER NOT NULL DEFAULT 0,
    "tier_turns" JSONB NOT NULL DEFAULT '{}',
    "baseline_models" JSONB NOT NULL DEFAULT '{}',
    CONSTRAINT "LiteLLM_AutoRouterUserSession_pkey" PRIMARY KEY ("user_id", "api_key", "session_id", "router_name")
);
"#;

const INSERT_SESSION_SQL: &str = r#"INSERT INTO "LiteLLM_AutoRouterSession"
    (api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model,
     turns, spend, saved_spend, savings_estimated_turns, savings_estimated_actual_spend,
     savings_estimated_saved_spend, classifier_cost, classifier_cost_recorded_turns, tier_turns)
    VALUES ($1, $2, $3, $4, $5::timestamp, $6::timestamp, $7, $8,
        ($9::text::numeric)::double precision, $10, $11, $12, $13, $14, $15, ($16::text)::jsonb)"#;

const INSERT_USER_SESSION_SQL: &str = r#"INSERT INTO "LiteLLM_AutoRouterUserSession"
    (user_id, api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model,
     turns, spend, saved_spend, savings_estimated_turns, savings_estimated_actual_spend,
     savings_estimated_saved_spend, classifier_cost, classifier_cost_recorded_turns, tier_turns)
    VALUES ($1, $2, $3, $4, $5, $6::timestamp, $7::timestamp, $8, $9,
        ($10::text::numeric)::double precision, $11, $12, $13, $14, $15, $16, ($17::text)::jsonb)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Database {
    fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
}

impl Database {
    async fn start() -> Self {
        let url = Environment::parse_from(["auto-router-read-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
                .resolve()
                .await
                .expect("the database fixture");
        let manager = Manager::new(
            fixture
                .options
                .url
                .expose_secret()
                .parse()
                .expect("the database url"),
            NoTls,
        );
        let pool = Pool::builder(manager)
            .max_size(2)
            .build()
            .expect("the database pool");
        Self {
            fixture,
            store: PostgresStore {
                pool: Some(pool.clone()),
            },
            pool,
        }
    }

    fn reads(&self) -> PgAutoRouterReads {
        PgAutoRouterReads {
            store: PostgresStore::clone(&self.store),
            availability: Availability::default(),
        }
    }

    async fn execute(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(statement, parameters)
            .await
            .expect("the statement runs");
    }

    async fn seed(&self, seed: &Seed<'_>, scope: Scope) {
        let spend = seed.spend.to_string();
        let shared: [&(dyn ToSql + Sync); 15] = [
            &seed.key,
            &seed.session,
            &seed.router,
            &seed.kind,
            &seed.first,
            &seed.last,
            &seed.model,
            &seed.turns,
            &spend,
            &seed.saved,
            &seed.estimate,
            &seed.actual,
            &seed.estimated_saved,
            &seed.classifier,
            &seed.recorded,
        ];
        let mut parameters: Vec<&(dyn ToSql + Sync)> = Vec::new();
        match scope {
            Scope::Session => parameters.extend(shared),
            Scope::User => parameters.push(&seed.user),
        }
        parameters.push(&seed.tiers);
        let statement = match scope {
            Scope::Session => INSERT_SESSION_SQL,
            Scope::User => INSERT_USER_SESSION_SQL,
        };
        self.execute(statement, &parameters).await;
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

enum Scope {
    Session,
    User,
}

struct Seed<'a> {
    user: &'a str,
    key: &'a str,
    session: &'a str,
    router: &'a str,
    kind: &'a str,
    first: &'a str,
    last: &'a str,
    model: &'a str,
    turns: i32,
    spend: rust_decimal::Decimal,
    saved: f64,
    estimate: i32,
    actual: f64,
    estimated_saved: f64,
    classifier: f64,
    recorded: i32,
    tiers: &'a str,
}

fn complexity_seed() -> Seed<'static> {
    Seed {
        user: "",
        key: "hash-a",
        session: "s1",
        router: "complexity-auto",
        kind: "complexity",
        first: "2024-01-01T00:00:00",
        last: "2024-01-01T00:01:00",
        model: "gpt-4o",
        turns: 4,
        spend: money("2.0"),
        saved: 2.0,
        estimate: 4,
        actual: 2.0,
        estimated_saved: 2.0,
        classifier: 0.5,
        recorded: 4,
        tiers: r#"{"SIMPLE": 4}"#,
    }
}

fn day(month: u32, day_of_month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, month, day_of_month).unwrap()
}

fn window(start: &str, end: &str) -> super::BenchmarksQuery {
    query(Some(start), Some(end), None, None)
}

#[tokio::test]
async fn the_rollup_query_groups_compares_and_filters() {
    let database = Database::start().await;
    database.seed(&complexity_seed(), Scope::Session).await;
    database
        .seed(
            &Seed {
                key: "hash-b",
                session: "s2",
                turns: 2,
                spend: money("1.0"),
                saved: 1.0,
                estimate: 2,
                actual: 1.0,
                estimated_saved: 1.0,
                classifier: 0.25,
                recorded: 2,
                tiers: r#"{"COMPLEX": 2}"#,
                ..complexity_seed()
            },
            Scope::Session,
        )
        .await;
    database
        .seed(
            &Seed {
                session: "s3",
                router: "adaptive-auto",
                kind: "adaptive",
                turns: 2,
                spend: money("4.0"),
                saved: 4.0,
                estimate: 2,
                actual: 4.0,
                estimated_saved: 4.0,
                classifier: 0.5,
                recorded: 1,
                tiers: "{}",
                ..complexity_seed()
            },
            Scope::Session,
        )
        .await;
    database
        .seed(
            &Seed {
                session: "old",
                first: "2023-12-31T00:00:00",
                last: "2023-12-31T00:01:00",
                ..complexity_seed()
            },
            Scope::Session,
        )
        .await;
    database
        .seed(
            &Seed {
                user: "u1",
                key: "",
                session: "us1",
                router: "quality-auto",
                kind: "quality",
                turns: 3,
                spend: money("1.5"),
                saved: 1.5,
                estimate: 3,
                actual: 1.5,
                estimated_saved: 1.5,
                classifier: 0.25,
                recorded: 3,
                tiers: "{}",
                ..complexity_seed()
            },
            Scope::User,
        )
        .await;
    let reads = database.reads();
    let answer = benchmarks_for(&reads, &window("2024-01-01", "2024-01-01"), &[], day(1, 5))
        .await
        .unwrap();
    assert_eq!(answer.groups.len(), 3);
    assert_eq!(answer.groups[0].router_name, "adaptive-auto");
    assert_eq!(answer.groups[0].spend, money("4.0"));
    assert_eq!(answer.groups[0].savings_classifier_cost, None);
    assert_eq!(answer.groups[2].router_name, "quality-auto");
    assert_eq!(answer.groups[2].turns, 3);
    assert_eq!(answer.groups[1].router_name, "complexity-auto");
    assert_eq!(answer.groups[1].sessions, 2);
    assert_eq!(answer.groups[1].turns, 6);
    assert_eq!(answer.groups[1].tier_turns.get("SIMPLE"), Some(&4));
    assert_eq!(answer.groups[1].tier_turns.get("COMPLEX"), Some(&2));
    assert_eq!(answer.groups[1].classifier_cost, Some(money("0.75")));
    assert_eq!(
        answer.groups[1].savings_classifier_cost,
        Some(money("0.75"))
    );
    assert_eq!(answer.groups[1].saved_spend, Some(money("3.0")));
    assert_eq!(answer.groups[1].baseline_spend, Some(money("6.0")));
    assert_eq!(answer.groups[1].saved_pct, Some(money("50")));
    let mine = benchmarks_for(
        &reads,
        &super::BenchmarksQuery {
            user_id: Some("u1".to_owned()),
            ..window("2024-01-01", "2024-01-01")
        },
        &[],
        day(1, 5),
    )
    .await
    .unwrap();
    assert_eq!(mine.groups.len(), 1);
    assert_eq!(mine.groups[0].router_name, "quality-auto");
    assert_eq!(mine.groups[0].turns, 3);
    let keyed = benchmarks_for(
        &reads,
        &super::BenchmarksQuery {
            key_hash: Some("hash-b".to_owned()),
            ..window("2024-01-01", "2024-01-01")
        },
        &[],
        day(1, 5),
    )
    .await
    .unwrap();
    assert_eq!(keyed.groups.len(), 1);
    assert_eq!(keyed.groups[0].turns, 2);
    let past = benchmarks_for(&reads, &window("2023-12-31", "2023-12-31"), &[], day(1, 5))
        .await
        .unwrap();
    assert_eq!(past.groups.len(), 1);
    assert_eq!(past.groups[0].router_name, "complexity-auto");
    assert_eq!(past.groups[0].turns, 4);
    database.close().await;
}

#[tokio::test]
async fn the_latest_session_reads_the_newest_row_under_the_key() {
    let database = Database::start().await;
    database.seed(&complexity_seed(), Scope::Session).await;
    database
        .seed(
            &Seed {
                router: "adaptive-auto",
                kind: "adaptive",
                last: "2024-01-02T00:00:00",
                model: "gpt-4o-mini",
                ..complexity_seed()
            },
            Scope::Session,
        )
        .await;
    database
        .seed(
            &Seed {
                key: "hash-b",
                model: "o3",
                ..complexity_seed()
            },
            Scope::Session,
        )
        .await;
    let reads = database.reads();
    let newest = reads
        .latest_session("hash-a", "s1")
        .await
        .unwrap()
        .expect("the session under hash-a");
    assert_eq!(newest.router_name, "adaptive-auto");
    assert_eq!(newest.last_model, "gpt-4o-mini");
    assert_eq!(
        reads
            .latest_session("hash-b", "s1")
            .await
            .unwrap()
            .map(|row| row.last_model),
        Some("o3".to_owned())
    );
    assert!(
        reads
            .latest_session("hash-c", "s1")
            .await
            .unwrap()
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
async fn a_store_without_a_pool_reports_the_database_failure() {
    let reads = PgAutoRouterReads {
        store: PostgresStore { pool: None },
        availability: Availability::default(),
    };
    let window =
        BenchmarkWindow::resolve(Some("2024-01-01"), Some("2024-01-01"), day(1, 5)).unwrap();
    let error = reads.agg_rows(&window, None, None).await.unwrap_err();
    assert_eq!(error.status_code(), 500);
    assert!(error.to_string().contains("No DB Connected"));
    let error = reads.latest_session("hash-a", "s1").await.unwrap_err();
    assert_eq!(error.status_code(), 500);
}

fn money(text: &str) -> rust_decimal::Decimal {
    text.parse().unwrap()
}
