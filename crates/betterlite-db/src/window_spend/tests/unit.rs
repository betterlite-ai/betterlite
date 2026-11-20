use std::time::Duration;

use rust_decimal::Decimal;

use crate::postgres_store::session::PostgresStore;
use crate::window_queue::WindowQueueError;
use crate::window_spend::error::WindowSpendError;
use crate::window_spend::seed::{SeedEntity, exclusion_upper_bound, seed_base, window_instant};
use crate::window_spend::store::{
    DEFAULT_TRANSACTION_TIMEOUT, WindowSpendStore, ordered_transactions,
};
use crate::window_spend::tests::support::{
    Increment, decimal, key_transaction, naive, totals, transaction, window_transaction,
};
use crate::window_spend::upsert::{primary_key, upsert_params};

#[test]
fn seed_base_subtracts_the_batch_and_floors_at_the_before_batch_sum() {
    let discounted = seed_base(&totals("12.5", "3.25"), decimal("2.5")).expect("a seed base");
    assert_eq!(discounted, decimal("10"));
    let floored = seed_base(&totals("12.5", "11.5"), decimal("2.5")).expect("a seed base");
    assert_eq!(floored, decimal("11.5"));
    let empty = seed_base(&totals("0", "0"), decimal("0")).expect("a seed base");
    assert_eq!(empty, Decimal::ZERO);
}

#[test]
fn seed_base_reports_an_underflow() {
    let underflowing = seed_base(&totals(&Decimal::MIN.to_string(), "0"), decimal("1"));
    assert!(matches!(underflowing, Err(WindowSpendError::Overflow)));
}

#[test]
fn exclusion_upper_bound_floors_to_the_whole_second() {
    assert_eq!(
        exclusion_upper_bound(naive("2026-01-01T00:00:00.000900")),
        naive("2026-01-01T00:00:00")
    );
    assert_eq!(
        exclusion_upper_bound(naive("2026-01-01T00:00:00.000000")),
        naive("2026-01-01T00:00:00")
    );
    assert_eq!(
        exclusion_upper_bound(naive("2026-01-01T23:59:59.999999")),
        naive("2026-01-01T23:59:59")
    );
}

#[test]
fn window_instant_reads_naive_stamps_as_written() {
    assert_eq!(
        window_instant("2026-01-01T00:00:00.000000").expect("an instant"),
        naive("2026-01-01T00:00:00")
    );
    assert_eq!(
        window_instant("2026-01-01 00:05:00").expect("an instant"),
        naive("2026-01-01T00:05:00")
    );
    assert_eq!(
        window_instant("2026-01-01").expect("an instant"),
        naive("2026-01-01T00:00:00")
    );
}

#[test]
fn window_instant_converts_an_offset_to_utc() {
    assert_eq!(
        window_instant("2026-01-01T02:00:00.000000+02:00").expect("an instant"),
        naive("2026-01-01T00:00:00")
    );
    assert_eq!(
        window_instant("2025-12-31T22:00:00.000000-05:00").expect("an instant"),
        naive("2026-01-01T03:00:00")
    );
}

#[test]
fn window_instant_rejects_text_that_is_not_a_stamp() {
    let malformed = window_instant("soon");
    assert!(matches!(
        malformed,
        Err(WindowSpendError::Stamp(
            WindowQueueError::InvalidStamp { .. }
        ))
    ));
}

#[test]
fn only_keys_and_teams_can_seed_from_spend_logs() {
    assert_eq!(SeedEntity::from_litellm_name("key"), Some(SeedEntity::Key));
    assert_eq!(
        SeedEntity::from_litellm_name("team"),
        Some(SeedEntity::Team)
    );
    assert_eq!(SeedEntity::from_litellm_name("team_member"), None);
    assert_eq!(SeedEntity::from_litellm_name("user"), None);
    assert_eq!(SeedEntity::from_litellm_name("org"), None);
    assert_eq!(SeedEntity::from_litellm_name("project"), None);
    assert_eq!(SeedEntity::from_litellm_name("end_user"), None);
    assert_eq!(SeedEntity::from_litellm_name(""), None);
}

#[test]
fn upsert_params_add_the_seed_to_the_increment() {
    let queued = transaction(&Increment {
        entity_type: "key",
        entity_id: "k1",
        window_duration: "1h",
        window_start: "2026-01-01T00:00:00.000000",
        spend: "2.5",
        started_at: Some("2026-01-01T00:00:30.000000"),
    });
    let params = upsert_params(&queued, decimal("10"), naive("2026-01-01T00:05:00.000000"))
        .expect("upsert params");
    assert_eq!(params.entity_type, "key");
    assert_eq!(params.entity_id, "k1");
    assert_eq!(params.window_duration, "1h");
    assert_eq!(params.window_start, naive("2026-01-01T00:00:00"));
    assert_eq!(params.spend, decimal("12.5"));
    assert_eq!(params.increment, decimal("2.5"));
    assert_eq!(params.now, naive("2026-01-01T00:05:00"));
}

#[test]
fn upsert_params_convert_an_aware_window_start() {
    let queued = key_transaction("k1", "2026-01-01T02:00:00.000000+02:00", "1");
    let params = upsert_params(&queued, Decimal::ZERO, naive("2026-01-01T00:05:00.000000"))
        .expect("upsert params");
    assert_eq!(params.window_start, naive("2026-01-01T00:00:00"));
    assert_eq!(params.spend, decimal("1"));
}

#[test]
fn upsert_params_report_an_overflow() {
    let queued = key_transaction("k1", "2026-01-01T00:00:00.000000", "1");
    let params = upsert_params(&queued, Decimal::MAX, naive("2026-01-01T00:05:00.000000"));
    assert!(matches!(params, Err(WindowSpendError::Overflow)));
}

#[test]
fn upsert_params_reject_a_malformed_window_start() {
    let queued = key_transaction("k1", "soon", "1");
    let params = upsert_params(&queued, Decimal::ZERO, naive("2026-01-01T00:05:00.000000"));
    assert!(matches!(params, Err(WindowSpendError::Stamp(_))));
}

#[test]
fn the_default_transaction_timeout_is_sixty_seconds() {
    let store = WindowSpendStore::from_store(PostgresStore { pool: None });
    assert_eq!(DEFAULT_TRANSACTION_TIMEOUT, Duration::from_secs(60));
    assert_eq!(store.transaction_timeout, DEFAULT_TRANSACTION_TIMEOUT);
}

#[test]
fn the_primary_key_is_the_entity_and_its_window_duration() {
    let queued = transaction(&Increment {
        entity_type: "team",
        entity_id: "t1",
        window_duration: "30d",
        window_start: "2026-01-01T00:00:00.000000",
        spend: "1",
        started_at: None,
    });
    assert_eq!(
        primary_key(&queued),
        ("team".to_owned(), "t1".to_owned(), "30d".to_owned())
    );
}

#[test]
fn ordered_transactions_sort_by_primary_key_then_window_start() {
    let transactions = vec![
        key_transaction("k2", "2026-01-01T00:00:00.000000", "1"),
        key_transaction("k1", "2026-01-01T01:00:00.000000", "1"),
        window_transaction("team", "t1", "1h", "2026-01-01T00:00:00.000000", "1"),
        key_transaction("k1", "2026-01-01T00:00:00.000000", "1"),
        window_transaction("key", "k1", "1d", "2026-01-01T00:00:00.000000", "1"),
    ];
    let ordered: Vec<_> = ordered_transactions(&transactions)
        .iter()
        .map(|queued| {
            (
                queued.entity_type.as_str(),
                queued.entity_id.as_str(),
                queued.window_duration.as_str(),
                queued.window_start.as_str(),
            )
        })
        .collect();
    assert_eq!(
        ordered,
        vec![
            ("key", "k1", "1d", "2026-01-01T00:00:00.000000"),
            ("key", "k1", "1h", "2026-01-01T00:00:00.000000"),
            ("key", "k1", "1h", "2026-01-01T01:00:00.000000"),
            ("key", "k2", "1h", "2026-01-01T00:00:00.000000"),
            ("team", "t1", "1h", "2026-01-01T00:00:00.000000"),
        ]
    );
}
