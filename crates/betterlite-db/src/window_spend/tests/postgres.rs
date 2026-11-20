use std::slice::from_ref;
use std::sync::Arc;

use rstest::rstest;
use tokio::sync::Barrier;

use crate::postgres_store::session::PostgresStore;
use crate::window_spend::error::WindowSpendError;
use crate::window_spend::sql::SqlWindowSpendLogs;
use crate::window_spend::store::{WindowRoll, WindowSpendStore};
use crate::window_spend::tests::support::{
    BarrierLogs, CountingLogs, INSERT_WINDOW_SQL, Increment, StaticLogs, close, database, decimal,
    execute, insert_log, key_transaction, log_store, naive, store, stored_window, totals,
    transaction, utc,
};

#[tokio::test]
async fn a_missing_row_gets_the_seed_plus_the_increment() {
    let (fixture, pool) = database().await;
    let logs = StaticLogs {
        totals: totals("10", "4"),
    };
    let queued = transaction(&Increment {
        entity_type: "key",
        entity_id: "k1",
        window_duration: "1h",
        window_start: "2026-01-01T00:00:00.000000",
        spend: "2.5",
        started_at: Some("2026-01-01T00:00:30.000000"),
    });
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("10"));
    assert_eq!(stored.window_start, naive("2026-01-01T00:00:00"));
    assert_eq!(stored.created_at, naive("2026-01-01T00:05:00"));
    assert_eq!(stored.updated_at, naive("2026-01-01T00:05:00"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn an_existing_row_takes_only_the_increment() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        INSERT_WINDOW_SQL,
        &[&"k1", &naive("2026-01-01T00:00:00.000"), &decimal("2.5")],
    )
    .await;
    let logs = CountingLogs::with_totals(totals("100", "100"));
    let queued = key_transaction("k1", "2026-01-01T00:00:00.000000", "1");
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    assert_eq!(logs.calls(), 0);
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("3.5"));
    assert_eq!(stored.window_start, naive("2026-01-01T00:00:00"));
    assert_eq!(stored.updated_at, naive("2026-01-01T00:05:00"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn an_older_stored_window_is_replaced_by_the_increment() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        INSERT_WINDOW_SQL,
        &[&"k1", &naive("2025-12-31T23:00:00.000"), &decimal("9")],
    )
    .await;
    let logs = CountingLogs::with_totals(totals("100", "100"));
    let queued = key_transaction("k1", "2026-01-01T00:00:00.000000", "2.5");
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    assert_eq!(logs.calls(), 0);
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("2.5"));
    assert_eq!(stored.window_start, naive("2026-01-01T00:00:00"));
    assert_eq!(stored.updated_at, naive("2026-01-01T00:05:00"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_late_increment_lands_in_the_newer_window() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        INSERT_WINDOW_SQL,
        &[&"k1", &naive("2026-01-01T01:00:00.000"), &decimal("2")],
    )
    .await;
    let logs = CountingLogs::with_totals(totals("100", "100"));
    let queued = key_transaction("k1", "2026-01-01T00:00:00.000000", "1");
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T01:00:05.000000"))
        .await
        .expect("the committed window");
    assert_eq!(logs.calls(), 0);
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("3"));
    assert_eq!(stored.window_start, naive("2026-01-01T01:00:00"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn concurrent_seeders_add_the_seed_once() {
    let (fixture, pool) = database().await;
    let logs = Arc::new(BarrierLogs {
        totals: totals("10", "4"),
        barrier: Arc::new(Barrier::new(2)),
    });
    let first = key_transaction("k1", "2026-01-01T00:00:00.000000", "1");
    let second = key_transaction("k1", "2026-01-01T00:00:00.000000", "1");
    let writer = store(&pool);
    let now = utc("2026-01-01T00:05:00.000000");
    let (left, right) = tokio::join!(
        writer.commit_window_spend_updates(from_ref(&first), logs.as_ref(), now),
        writer.commit_window_spend_updates(from_ref(&second), logs.as_ref(), now),
    );
    left.expect("the first seeder");
    right.expect("the second seeder");
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("11"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn other_entity_types_have_no_seed() {
    let (fixture, pool) = database().await;
    let logs = CountingLogs::with_totals(totals("10", "4"));
    let queued = transaction(&Increment {
        entity_type: "user",
        entity_id: "u1",
        window_duration: "1h",
        window_start: "2026-01-01T00:00:00.000000",
        spend: "2.5",
        started_at: Some("2026-01-01T00:00:30.000000"),
    });
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    assert_eq!(logs.calls(), 0);
    let stored = stored_window(&pool, "user", "u1", "1h").await;
    assert_eq!(stored.spend, decimal("2.5"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn seeds_from_the_spend_logs_since_the_window_start() {
    let (fixture, pool) = database().await;
    insert_log(&pool, "k1", None, "3", "2026-01-01T00:00:10.000").await;
    insert_log(&pool, "k1", None, "1", "2026-01-01T00:02:00.000").await;
    insert_log(&pool, "k1", None, "2", "2025-12-31T23:59:00.000").await;
    insert_log(&pool, "other", None, "7", "2026-01-01T00:00:20.000").await;
    let logs = SqlWindowSpendLogs {
        store: log_store(&pool),
    };
    let queued = transaction(&Increment {
        entity_type: "key",
        entity_id: "k1",
        window_duration: "1h",
        window_start: "2026-01-01T00:00:00.000000",
        spend: "1.5",
        started_at: Some("2026-01-01T00:01:00.000000"),
    });
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    assert_eq!(
        stored_window(&pool, "key", "k1", "1h").await.spend,
        decimal("4.5")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_seed_counts_the_whole_total_without_a_batch_start() {
    let (fixture, pool) = database().await;
    insert_log(&pool, "k2", None, "2", "2026-01-01T00:00:10.000").await;
    insert_log(&pool, "k2", None, "3", "2026-01-01T00:03:00.000").await;
    let logs = SqlWindowSpendLogs {
        store: log_store(&pool),
    };
    let queued = key_transaction("k2", "2026-01-01T00:00:00.000000", "1");
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    assert_eq!(
        stored_window(&pool, "key", "k2", "1h").await.spend,
        decimal("6")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_team_seeds_from_its_own_logs() {
    let (fixture, pool) = database().await;
    insert_log(&pool, "ignored", Some("t1"), "2", "2026-01-01T00:00:10.000").await;
    insert_log(&pool, "ignored", Some("t2"), "5", "2026-01-01T00:00:10.000").await;
    let logs = SqlWindowSpendLogs {
        store: log_store(&pool),
    };
    let queued = transaction(&Increment {
        entity_type: "team",
        entity_id: "t1",
        window_duration: "1h",
        window_start: "2026-01-01T00:00:00.000000",
        spend: "1",
        started_at: None,
    });
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    assert_eq!(
        stored_window(&pool, "team", "t1", "1h").await.spend,
        decimal("3")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn an_empty_log_table_seeds_zero() {
    let (fixture, pool) = database().await;
    let logs = SqlWindowSpendLogs {
        store: log_store(&pool),
    };
    let queued = key_transaction("k3", "2026-01-01T00:00:00.000000", "2.5");
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    assert_eq!(
        stored_window(&pool, "key", "k3", "1h").await.spend,
        decimal("2.5")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_seed_exclusion_floors_the_batch_start_to_the_second() {
    let (fixture, pool) = database().await;
    insert_log(&pool, "k4", None, "5", "2026-01-01T00:00:00.000").await;
    insert_log(&pool, "k4", None, "1", "2026-01-01T00:00:30.000").await;
    let logs = SqlWindowSpendLogs {
        store: log_store(&pool),
    };
    let queued = transaction(&Increment {
        entity_type: "key",
        entity_id: "k4",
        window_duration: "1h",
        window_start: "2026-01-01T00:00:00.000000",
        spend: "10",
        started_at: Some("2026-01-01T00:00:00.000900"),
    });
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    assert_eq!(
        stored_window(&pool, "key", "k4", "1h").await.spend,
        decimal("10")
    );
    close(fixture, pool).await;
}

#[rstest]
#[case::existing("existing", "1h", "2")]
#[case::fresh_hourly("fresh", "1h", "5")]
#[case::fresh_daily("fresh", "1d", "5")]
#[tokio::test]
async fn a_batch_lands_a_row_for_each_primary_key(
    #[case] entity_id: &str,
    #[case] window_duration: &str,
    #[case] expected: &str,
) {
    let (fixture, pool) = database().await;
    let inserted_at = naive("2026-01-01T00:00:00.000");
    execute(
        &pool,
        INSERT_WINDOW_SQL,
        &[&"existing", &inserted_at, &decimal("1")],
    )
    .await;
    let logs = StaticLogs {
        totals: totals("5", "0"),
    };
    let existing = key_transaction("existing", "2026-01-01T00:00:00.000000", "1");
    let fresh = key_transaction("fresh", "2026-01-01T00:00:00.000000", "2");
    let daily = transaction(&Increment {
        entity_type: "key",
        entity_id: "fresh",
        window_duration: "1d",
        window_start: "2026-01-01T00:00:00.000000",
        spend: "1",
        started_at: None,
    });
    store(&pool)
        .commit_window_spend_updates(
            &[existing, fresh, daily],
            &logs,
            utc("2026-01-01T00:05:00.000000"),
        )
        .await
        .expect("the committed windows");
    let stored = stored_window(&pool, "key", entity_id, window_duration).await;
    assert_eq!(stored.spend, decimal(expected));
    close(fixture, pool).await;
}

#[tokio::test]
async fn an_aware_window_stamp_is_stored_as_utc() {
    let (fixture, pool) = database().await;
    let logs = StaticLogs {
        totals: totals("1", "0"),
    };
    let queued = key_transaction("k1", "2026-01-01T02:00:00.000000+02:00", "1");
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("the committed window");
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.window_start, naive("2026-01-01T00:00:00"));
    assert_eq!(stored.spend, decimal("1"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_malformed_window_stamp_writes_no_row() {
    let (fixture, pool) = database().await;
    let logs = StaticLogs {
        totals: totals("0", "0"),
    };
    let queued = key_transaction("k1", "soon", "1");
    let failed = store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await;
    assert!(matches!(failed, Err(WindowSpendError::Stamp(_))));
    let client = pool.get().await.expect("the database connection");
    let written = client
        .query_opt(
            r#"SELECT 1 FROM "LiteLLM_BudgetWindowSpend" WHERE entity_type = 'key' AND entity_id = 'k1' AND window_duration = '1h'"#,
            &[],
        )
        .await
        .expect("the window query");
    assert!(written.is_none());
    close(fixture, pool).await;
}

#[tokio::test]
async fn nothing_to_commit_touches_no_database() {
    let writer = WindowSpendStore::from_store(PostgresStore { pool: None });
    let logs = StaticLogs {
        totals: totals("0", "0"),
    };
    writer
        .commit_window_spend_updates(&[], &logs, utc("2026-01-01T00:05:00.000000"))
        .await
        .expect("an empty batch");
    let queued = key_transaction("k1", "2026-01-01T00:00:00.000000", "1");
    let failed = writer
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T00:05:00.000000"))
        .await;
    assert!(matches!(failed, Err(WindowSpendError::Database(_))));
}

#[tokio::test]
async fn a_roll_advances_and_zeroes_the_row() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        INSERT_WINDOW_SQL,
        &[&"k1", &naive("2026-01-01T00:00:00.000"), &decimal("9")],
    )
    .await;
    store(&pool)
        .roll_window_spend_row(&WindowRoll {
            entity_type: "key",
            entity_id: "k1",
            window_duration: "1h",
            new_window_start: naive("2026-01-01T01:00:00"),
            now: utc("2026-01-01T01:00:05.000000"),
        })
        .await
        .expect("the rolled window");
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("0"));
    assert_eq!(stored.window_start, naive("2026-01-01T01:00:00"));
    assert_eq!(stored.updated_at, naive("2026-01-01T01:00:05"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_roll_behind_the_stored_window_changes_nothing() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        INSERT_WINDOW_SQL,
        &[&"k1", &naive("2026-01-01T02:00:00.000"), &decimal("9")],
    )
    .await;
    store(&pool)
        .roll_window_spend_row(&WindowRoll {
            entity_type: "key",
            entity_id: "k1",
            window_duration: "1h",
            new_window_start: naive("2026-01-01T01:00:00"),
            now: utc("2026-01-01T01:00:05.000000"),
        })
        .await
        .expect("the rolled window");
    store(&pool)
        .roll_window_spend_row(&WindowRoll {
            entity_type: "key",
            entity_id: "k1",
            window_duration: "1h",
            new_window_start: naive("2026-01-01T02:00:00"),
            now: utc("2026-01-01T02:00:05.000000"),
        })
        .await
        .expect("the rolled window");
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("9"));
    assert_eq!(stored.window_start, naive("2026-01-01T02:00:00"));
    assert_eq!(stored.updated_at, naive("2025-12-31T23:00:00"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn an_increment_before_the_roll_survives_it() {
    let (fixture, pool) = database().await;
    let logs = StaticLogs {
        totals: totals("2", "0"),
    };
    let queued = key_transaction("k1", "2026-01-01T01:00:00.000000", "2");
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T01:00:02.000000"))
        .await
        .expect("the committed window");
    store(&pool)
        .roll_window_spend_row(&WindowRoll {
            entity_type: "key",
            entity_id: "k1",
            window_duration: "1h",
            new_window_start: naive("2026-01-01T01:00:00"),
            now: utc("2026-01-01T01:00:05.000000"),
        })
        .await
        .expect("the rolled window");
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("2"));
    assert_eq!(stored.window_start, naive("2026-01-01T01:00:00"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_roll_before_the_increment_keeps_the_increment() {
    let (fixture, pool) = database().await;
    execute(
        &pool,
        INSERT_WINDOW_SQL,
        &[&"k1", &naive("2026-01-01T00:00:00.000"), &decimal("9")],
    )
    .await;
    store(&pool)
        .roll_window_spend_row(&WindowRoll {
            entity_type: "key",
            entity_id: "k1",
            window_duration: "1h",
            new_window_start: naive("2026-01-01T01:00:00"),
            now: utc("2026-01-01T01:00:00.000000"),
        })
        .await
        .expect("the rolled window");
    let logs = CountingLogs::with_totals(totals("100", "100"));
    let queued = key_transaction("k1", "2026-01-01T01:00:00.000000", "2");
    store(&pool)
        .commit_window_spend_updates(&[queued], &logs, utc("2026-01-01T01:00:05.000000"))
        .await
        .expect("the committed window");
    assert_eq!(logs.calls(), 0);
    let stored = stored_window(&pool, "key", "k1", "1h").await;
    assert_eq!(stored.spend, decimal("2"));
    assert_eq!(stored.window_start, naive("2026-01-01T01:00:00"));
    close(fixture, pool).await;
}
