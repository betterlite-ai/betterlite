#![cfg(test)]

use std::collections::BTreeMap;

use chrono::NaiveDate;

use super::support::{
    DIGEST, OTHER_DIGEST, close, database, dates, insert_spend_log, keys, moment, reads, recover,
};
use super::window::spend_logs_window;

#[tokio::test]
async fn window_boundaries_include_the_start_moment() {
    let database = database().await;
    insert_spend_log(
        &database.store,
        DIGEST,
        moment(2026, 1, 4, 0, 0),
        "at-start",
        None,
    )
    .await;
    insert_spend_log(
        &database.store,
        OTHER_DIGEST,
        moment(2026, 1, 3, 23, 59),
        "before-start",
        None,
    )
    .await;
    let window = spend_logs_window(&dates(&["2026-01-05"]));
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST, OTHER_DIGEST]),
        window,
    )
    .await;
    assert_eq!(
        recovered.get(DIGEST).unwrap().key_alias.as_deref(),
        Some("at-start")
    );
    assert!(!recovered.contains_key(OTHER_DIGEST));
    close(database).await;
}

#[tokio::test]
async fn window_boundaries_exclude_the_end_moment() {
    let database = database().await;
    insert_spend_log(
        &database.store,
        DIGEST,
        moment(2026, 1, 7, 0, 0),
        "at-end",
        None,
    )
    .await;
    insert_spend_log(
        &database.store,
        OTHER_DIGEST,
        moment(2026, 1, 6, 23, 59),
        "before-end",
        None,
    )
    .await;
    let window = spend_logs_window(&dates(&["2026-01-05"]));
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST, OTHER_DIGEST]),
        window,
    )
    .await;
    assert!(!recovered.contains_key(DIGEST));
    assert_eq!(
        recovered.get(OTHER_DIGEST).unwrap().key_alias.as_deref(),
        Some("before-end")
    );
    close(database).await;
}

#[tokio::test]
async fn window_boundaries_cover_the_day_after_the_latest_date() {
    let database = database().await;
    insert_spend_log(
        &database.store,
        DIGEST,
        moment(2026, 1, 6, 0, 0),
        "next-day",
        None,
    )
    .await;
    let window = spend_logs_window(&dates(&["2026-01-05"]));
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        window,
    )
    .await;
    assert_eq!(
        recovered.get(DIGEST).unwrap().key_alias.as_deref(),
        Some("next-day")
    );
    close(database).await;
}

#[tokio::test]
async fn window_boundaries_skip_spend_logs_for_an_empty_or_unparsable_date_set() {
    let database = database().await;
    insert_spend_log(
        &database.store,
        DIGEST,
        moment(2026, 1, 5, 10, 0),
        "spend-alias",
        None,
    )
    .await;
    let empty = spend_logs_window(&dates(&[]));
    let unparsable = spend_logs_window(&dates(&["not-a-date"]));
    assert!(empty.is_none());
    assert!(unparsable.is_none());
    let from_empty = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        empty,
    )
    .await;
    let from_unparsable = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        unparsable,
    )
    .await;
    assert!(from_empty.is_empty());
    assert!(from_unparsable.is_empty());
    close(database).await;
}

#[tokio::test]
async fn window_boundaries_keep_a_fractional_pre_epoch_start() {
    let database = database().await;
    let started = NaiveDate::from_ymd_opt(1969, 12, 29)
        .unwrap()
        .and_hms_milli_opt(0, 0, 0, 600)
        .unwrap();
    insert_spend_log(&database.store, DIGEST, started, "pre-epoch-start", None).await;
    let window = spend_logs_window(&dates(&["1969-12-30T00:00:00.500000"]));
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        window,
    )
    .await;
    assert_eq!(
        recovered.get(DIGEST).unwrap().key_alias.as_deref(),
        Some("pre-epoch-start")
    );
    close(database).await;
}
