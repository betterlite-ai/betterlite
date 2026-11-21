use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::support::{Journal, JournalBatch, Memory, ScriptedCache, money, reseed_over};
use crate::current_spend::{CurrentSpend, SpendQuery, WindowDescriptors};
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, SpendRow, SpendRowLookup, WindowEntity, WindowSpendRow,
};
use crate::spend_reseed::sql::{SpendRowsError, SqlSpendCounterRows};
use crate::spend_reseed::values::ReseedCacheValue;

const SCHEMA: &str = r#"CREATE TABLE "LiteLLM_VerificationToken" (token TEXT PRIMARY KEY, spend DOUBLE PRECISION);
INSERT INTO "LiteLLM_VerificationToken" (token, spend) VALUES ('racing', 9);"#;
const RESET: &str = r#"UPDATE "LiteLLM_VerificationToken" SET spend = 0 WHERE token = 'racing'"#;
const COUNTER: &str = "spend:key:racing";
const MARKER: &str = "spend_db_floor:spend:key:racing";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

// the budget reset commits between this read and its return, and writes the post-reset floor marker
struct ResetDuringRead {
    inner: SqlSpendCounterRows,
    pool: Pool,
    memory: Memory,
}

impl SpendCounterRows for ResetDuringRead {
    type Error = SpendRowsError;

    async fn spend_row(
        &self,
        lookup: SpendRowLookup<'_>,
    ) -> Result<Option<SpendRow>, SpendRowsError> {
        let stale = self.inner.spend_row(lookup).await;
        let client = self.pool.get().await.expect("the reset connection");
        client.execute(RESET, &[]).await.expect("the reset commit");
        let mut memory = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        memory.insert(MARKER.to_owned(), ReseedCacheValue::Number(Decimal::ZERO));
        stale
    }

    async fn window_row(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_duration: &str,
    ) -> Result<Option<WindowSpendRow>, SpendRowsError> {
        self.inner
            .window_row(entity, entity_id, window_duration)
            .await
    }

    async fn window_log_rows(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_start: DateTime<Utc>,
    ) -> Result<Vec<SpendAggregateRow>, SpendRowsError> {
        self.inner
            .window_log_rows(entity, entity_id, window_start)
            .await
    }
}

#[tokio::test]
async fn concurrent_reset_marker_wins() {
    let url = Environment::parse_from(["current-spend-postgres-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .expect("the database fixture");
    let address = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the database url");
    let pool = Pool::builder(Manager::new(address, NoTls))
        .max_size(2)
        .build()
        .expect("the database pool");
    let memory: Memory = Arc::new(Mutex::new(BTreeMap::from([(
        COUNTER.to_owned(),
        ReseedCacheValue::Number(money("5")),
    )])));
    let journal = Journal::default();
    let cache = ScriptedCache {
        redis: None,
        memory: Arc::clone(&memory),
        expiring: Mutex::new(Vec::new()),
        journal: Arc::clone(&journal),
    };
    let batch = JournalBatch {
        answer: Mutex::new(None),
        recorded: Mutex::new(Vec::new()),
        journal,
    };
    let rows = ResetDuringRead {
        inner: SqlSpendCounterRows::from(PostgresStore {
            pool: Some(pool.clone()),
        }),
        pool,
        memory,
    };
    let reseed = reseed_over(Some(rows));
    let service = CurrentSpend {
        cache: &cache,
        batch: &batch,
        reseed: &reseed,
        fail_closed_budget_enforcement: true,
    };
    let window = WindowDescriptors {
        entity_type: None,
        entity_id: None,
        duration: None,
        start: None,
    };
    let query = SpendQuery {
        counter_key: COUNTER,
        fallback_spend: Decimal::ZERO,
        max_budget: Some(money("10")),
        window,
        fallback_authoritative: false,
    };

    let spend = service.get_current_spend(&query).await.unwrap();
    assert_eq!(spend, money("5"));
    assert_eq!(
        cache.stored(MARKER),
        Some(ReseedCacheValue::Number(Decimal::ZERO))
    );
    assert_eq!(
        cache.stored(COUNTER),
        Some(ReseedCacheValue::Number(money("5")))
    );
    assert!(cache.expiring.lock().unwrap().is_empty());
    fixture.close_async().await.expect("the fixture cleanup");
}
