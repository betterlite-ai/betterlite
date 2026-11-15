use std::slice::from_ref;
use std::time::Duration;

use chrono::DateTime;
use rust_decimal::Decimal;

use super::fixtures::{ToolSpendCase, unreachable_store};
use super::{FixedJitter, REQUEST_TIME, RecordingPause, policy, transaction};
use crate::tool_spend::ToolSpendWriter as _;
use crate::tool_spend::flush_tool_usage_transactions;
use crate::tool_spend::index_rows::ToolIndexRow;
use crate::tool_spend::writer::{PostgresToolSpendWriter, ToolSpendWriteError};

const REJECT_BLOCKED_SQL: &str = r#"CREATE FUNCTION reject_blocked_tool() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.tool_name = 'blocked' THEN RAISE EXCEPTION 'blocked tool'; END IF; RETURN NEW; END $$;
CREATE TRIGGER reject_blocked_tool BEFORE INSERT OR UPDATE ON "LiteLLM_DailyToolSpend" FOR EACH ROW EXECUTE FUNCTION reject_blocked_tool();"#;
const SEED_ALPHA_SQL: &str = r#"INSERT INTO "LiteLLM_DailyToolSpend" (date, tool_name, spend, total_tokens, request_count, updated_at) VALUES ('2026-01-02', 'alpha', 1.0, 10, 1, now())"#;

fn request_time() -> chrono::NaiveDateTime {
    DateTime::from_timestamp(REQUEST_TIME, 0)
        .expect("a test instant")
        .naive_utc()
}

async fn flush_two_requests(case: &ToolSpendCase) {
    let writer = PostgresToolSpendWriter {
        store: case.store.clone(),
    };
    let transactions = [
        transaction(
            "req-1",
            "2026-01-02",
            &["search", "fetch"],
            Decimal::new(125, 2),
            42,
        ),
        transaction("req-2", "2026-01-03", &["fetch"], Decimal::new(75, 2), 8),
    ];
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    flush_tool_usage_transactions(
        &writer,
        &transactions,
        policy(2_000_000, 100, 3),
        &jitter,
        &pause,
    )
    .await
    .expect("the flush writes");
    assert!(pause.delays().is_empty());
}

fn expected_index_rows() -> [(String, String, chrono::NaiveDateTime); 3] {
    let start = request_time();
    [
        (String::from("req-1"), String::from("fetch"), start),
        (String::from("req-1"), String::from("search"), start),
        (String::from("req-2"), String::from("fetch"), start),
    ]
}

fn expected_daily_rollups() -> [(String, String, f64, i64, i64); 3] {
    [
        (
            String::from("2026-01-02"),
            String::from("fetch"),
            1.25_f64,
            42,
            1,
        ),
        (
            String::from("2026-01-02"),
            String::from("search"),
            1.25_f64,
            42,
            1,
        ),
        (
            String::from("2026-01-03"),
            String::from("fetch"),
            0.75_f64,
            8,
            1,
        ),
    ]
}

async fn flush_blocked_tool(case: &ToolSpendCase) -> Result<(), ToolSpendWriteError> {
    let writer = PostgresToolSpendWriter {
        store: case.store.clone(),
    };
    let transactions = [transaction(
        "req-1",
        "2026-01-02",
        &["alpha", "blocked"],
        Decimal::new(125, 2),
        42,
    )];
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let outcome = flush_tool_usage_transactions(
        &writer,
        &transactions,
        policy(2_000_000, 100, 3),
        &jitter,
        &pause,
    )
    .await;
    assert!(pause.delays().is_empty());
    outcome
}

#[tokio::test]
async fn flushes_index_rows_and_daily_rollups() {
    let case = ToolSpendCase::new().await;
    flush_two_requests(&case).await;
    assert_eq!(case.index_rows().await, expected_index_rows());
    assert_eq!(case.daily_rollups().await, expected_daily_rollups());
    case.close().await;
}

#[tokio::test]
async fn rolls_back_every_daily_increment_when_a_later_upsert_fails() {
    let case = ToolSpendCase::new().await;
    case.execute(REJECT_BLOCKED_SQL).await;
    case.execute(SEED_ALPHA_SQL).await;
    let outcome = flush_blocked_tool(&case).await;
    assert!(matches!(outcome, Err(ToolSpendWriteError::AfterSend(_))));
    let start = request_time();
    assert_eq!(
        case.index_rows().await,
        [
            (String::from("req-1"), String::from("alpha"), start),
            (String::from("req-1"), String::from("blocked"), start),
        ]
    );
    assert_eq!(
        case.daily_rollups().await,
        [(
            String::from("2026-01-02"),
            String::from("alpha"),
            1.0_f64,
            10,
            1
        )]
    );
    case.close().await;
}

#[tokio::test]
async fn retries_a_pool_that_cannot_connect_and_then_gives_up() {
    let writer = PostgresToolSpendWriter {
        store: unreachable_store(),
    };
    let transactions = [transaction(
        "req-1",
        "2026-01-02",
        &["search"],
        Decimal::new(125, 2),
        42,
    )];
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let outcome = flush_tool_usage_transactions(
        &writer,
        &transactions,
        policy(2_000_000, 100, 3),
        &jitter,
        &pause,
    )
    .await;
    assert!(matches!(outcome, Err(ToolSpendWriteError::Connect(_))));
    assert_eq!(
        pause.delays(),
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
}

#[tokio::test]
async fn an_index_statement_skips_a_duplicate_row() {
    let case = ToolSpendCase::new().await;
    let writer = PostgresToolSpendWriter {
        store: case.store.clone(),
    };
    let row = ToolIndexRow::new(
        String::from("req-1"),
        String::from("search"),
        DateTime::from_timestamp(REQUEST_TIME, 0).expect("a test instant"),
    );
    writer
        .write_index_rows(from_ref(&row))
        .await
        .expect("the first index write");
    writer
        .write_index_rows(from_ref(&row))
        .await
        .expect("a duplicate index row is skipped");
    assert_eq!(
        case.index_rows().await,
        [(
            String::from("req-1"),
            String::from("search"),
            request_time()
        )]
    );
    case.close().await;
}

#[tokio::test]
async fn empty_writes_never_reach_the_database() {
    let writer = PostgresToolSpendWriter {
        store: unreachable_store(),
    };
    writer
        .write_index_rows(&[])
        .await
        .expect("no statement to send");
    writer.apply_rollups(&[]).await.expect("no rollup to send");
}
