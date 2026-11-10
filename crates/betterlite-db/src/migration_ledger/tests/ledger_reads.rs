#[tokio::test]
async fn ledger_unavailable_none() {
    let absent = "20250101000000_absent";
    assert_eq!(
        failed_migration_logs(None, absent, FIXED_STARTED_AT).await,
        None
    );
    assert!(!failed_migration_recovered(None, absent, FIXED_STARTED_AT).await);
    let empty = SecretString::from("");
    assert_eq!(
        failed_migration_logs(Some(&empty), absent, FIXED_STARTED_AT).await,
        None
    );
    let unreachable = SecretString::from("postgresql://postgres:postgres@127.0.0.1:1/postgres");
    assert_eq!(
        failed_migration_logs(Some(&unreachable), absent, FIXED_STARTED_AT).await,
        None
    );
    assert!(!failed_migration_recovered(Some(&unreachable), absent, FIXED_STARTED_AT).await);
    let malformed = SecretString::from("postgresql://[::1/postgres");
    assert_eq!(
        failed_migration_logs(Some(&malformed), absent, FIXED_STARTED_AT).await,
        None
    );
}

#[tokio::test]
async fn ledger_absent_tuple() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let url = configured_url(&database);
        assert_eq!(
            failed_migration_logs(Some(url), "20250101000000_absent", FIXED_STARTED_AT).await,
            Some(String::new())
        );
        assert!(
            !failed_migration_recovered(Some(url), "20250101000000_absent", FIXED_STARTED_AT).await
        );
    })
    .await;
}

#[tokio::test]
async fn failed_logs_null_empty() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        insert_reading_rows(&database.pool).await;
        let url = configured_url(&database);
        assert_eq!(
            failed_migration_logs(Some(url), "20250107000000_null", FIXED_STARTED_AT).await,
            Some(String::new())
        );
        assert_eq!(
            failed_migration_logs(Some(url), "20250108000000_blank", FIXED_STARTED_AT).await,
            Some(String::new())
        );
        assert_eq!(
            failed_migration_logs(Some(url), "20250109000000_failed", FIXED_STARTED_AT).await,
            Some("boom".to_owned())
        );
    })
    .await;
}

#[tokio::test]
async fn recovered_identity_match() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let name = "20250110000000_identity";
        insert_read(&database.pool, "row-unfinished", name, Some("boom"), false).await;
        insert_read_later(&database.pool, "row-finished", name, Some("")).await;
        let url = configured_url(&database);
        assert_eq!(
            failed_migration_logs(Some(url), name, FIXED_STARTED_AT).await,
            Some("boom".to_owned())
        );
        assert!(!failed_migration_recovered(Some(url), name, FIXED_STARTED_AT).await);
        assert!(failed_migration_recovered(Some(url), name, LATER_STARTED_AT).await);
        assert_eq!(
            failed_migration_logs(Some(url), name, LATER_STARTED_AT).await,
            Some(String::new())
        );
    })
    .await;
}
