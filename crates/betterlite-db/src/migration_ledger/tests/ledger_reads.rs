fn found(lookup: FailedLookup) -> FailedMigration {
    match lookup {
        FailedLookup::Found(state) => state,
        FailedLookup::Unavailable | FailedLookup::Absent => {
            panic!("expected a ledger row, got {lookup:?}")
        }
    }
}

#[tokio::test]
async fn ledger_unavailable_none() {
    let absent = "20250101000000_absent";
    assert!(matches!(
        look_up_failed_migration(None, absent, FIXED_STARTED_AT).await,
        FailedLookup::Unavailable
    ));
    let empty = SecretString::from("");
    assert!(matches!(
        look_up_failed_migration(Some(&empty), absent, FIXED_STARTED_AT).await,
        FailedLookup::Unavailable
    ));
    let unreachable = SecretString::from("postgresql://postgres:postgres@127.0.0.1:1/postgres");
    assert!(matches!(
        look_up_failed_migration(Some(&unreachable), absent, FIXED_STARTED_AT).await,
        FailedLookup::Unavailable
    ));
    let malformed = SecretString::from("postgresql://[::1/postgres");
    assert!(matches!(
        look_up_failed_migration(Some(&malformed), absent, FIXED_STARTED_AT).await,
        FailedLookup::Unavailable
    ));
}

#[tokio::test]
async fn ledger_absent_tuple() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let url = configured_url(&database);
        assert!(matches!(
            look_up_failed_migration(Some(url), "20250101000000_absent", FIXED_STARTED_AT).await,
            FailedLookup::Absent
        ));
    })
    .await;
}

#[tokio::test]
async fn failed_logs_null_empty() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        insert_reading_rows(&database.pool).await;
        let url = configured_url(&database);
        let null_logs = found(
            look_up_failed_migration(Some(url), "20250107000000_null", FIXED_STARTED_AT).await,
        );
        assert!(null_logs.logs.is_empty());
        assert!(!null_logs.recovered);
        let blank_logs = found(
            look_up_failed_migration(Some(url), "20250108000000_blank", FIXED_STARTED_AT).await,
        );
        assert!(blank_logs.logs.is_empty());
        let kept = found(
            look_up_failed_migration(Some(url), "20250109000000_failed", FIXED_STARTED_AT).await,
        );
        assert_eq!(kept.logs, "boom");
        assert!(!kept.recovered);
    })
    .await;
}

#[tokio::test]
async fn recovered_identity_match() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let name = "20250110000000_identity";
        insert_read(&database.pool, "row-unfinished", name, Some("boom"), false).await;
        insert_read(&database.pool, "row-finished", name, Some(""), true).await;
        let url = configured_url(&database);
        let failed = found(look_up_failed_migration(Some(url), name, FIXED_STARTED_AT).await);
        assert_eq!(failed.logs, "boom");
        assert!(!failed.recovered);
        let resolved = found(look_up_failed_migration(Some(url), name, LATER_STARTED_AT).await);
        assert!(resolved.logs.is_empty());
        assert!(resolved.recovered);
    })
    .await;
}
