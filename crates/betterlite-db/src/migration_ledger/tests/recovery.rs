#[test]
fn completion_one_step() {
    let checksum = checksum_of("SELECT 1;");
    assert!(progress(1, "", &checksum).confirms_completion(b"SELECT 1;"));
    assert!(!progress(2, "", &checksum).confirms_completion(b"SELECT 1;"));
    assert!(!progress(0, "", &checksum).confirms_completion(b"SELECT 1;"));
}

#[test]
fn completion_empty_logs() {
    let checksum = checksum_of("SELECT 1;");
    assert!(progress(1, "", &checksum).confirms_completion(b"SELECT 1;"));
    assert!(progress(1, " \n\t ", &checksum).confirms_completion(b"SELECT 1;"));
    assert!(!progress(1, "failed", &checksum).confirms_completion(b"SELECT 1;"));
}

#[test]
fn completion_exact_checksum() {
    let script = "CREATE TABLE installed (id int);";
    let checksum = checksum_of(script);
    assert!(progress(1, "", &checksum).confirms_completion(script.as_bytes()));
    assert!(
        !progress(1, "", &checksum).confirms_completion(b"CREATE TABLE installed (id bigint);")
    );
    assert!(!progress(1, "", &"0".repeat(64)).confirms_completion(script.as_bytes()));
}

#[tokio::test]
async fn completed_checksum_match() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let installed = migration(
            "20250101000000_installed",
            "CREATE TABLE installed (id int);",
        );
        insert_confirmed(&database.pool, "row-installed", &installed, true).await;
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        let jitter = DrawingJitter(0);
        assert!(
            recover_completed_migration(&coordinator, PUBLIC_SCHEMA, &installed, &jitter)
                .await
                .unwrap()
        );
        let altered = migration(
            "20250101000000_installed",
            "CREATE TABLE installed (id bigint);",
        );
        assert!(
            !recover_completed_migration(&coordinator, PUBLIC_SCHEMA, &altered, &jitter)
                .await
                .unwrap()
        );
        coordinator.finish().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn completion_multiple_unfinished() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let pending = migration("20250102000000_pending", "CREATE TABLE pending (id int);");
        insert_failed(&database.pool, "row-first", &pending, FIXED_STARTED_AT).await;
        insert_failed(&database.pool, "row-second", &pending, LATER_STARTED_AT).await;
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        let jitter = DrawingJitter(0);
        assert!(
            !recover_completed_migration(&coordinator, PUBLIC_SCHEMA, &pending, &jitter)
                .await
                .unwrap()
        );
        coordinator.finish().await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| !row.get::<_, bool>("finished")));
    })
    .await;
}

#[tokio::test]
async fn completion_guard_rowcount() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let pending = migration("20250103000000_guarded", "CREATE TABLE guarded (id int);");
        insert_confirmed(&database.pool, "row-guarded", &pending, false).await;
        let client = database.pool.get().await.unwrap();
        client
            .batch_execute(
                "CREATE RULE suppress_completion AS ON UPDATE TO _prisma_migrations \
                 DO INSTEAD NOTHING",
            )
            .await
            .unwrap();
        drop(client);
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        let error =
            recover_completed_migration(&coordinator, PUBLIC_SCHEMA, &pending, &DrawingJitter(0))
                .await
                .expect_err("the guard cannot update the row");
        assert!(
            matches!(error, RecoveryError::CompletionMismatch),
            "{error:?}"
        );
        coordinator.finish().await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert_eq!(rows.len(), 1);
        assert!(!rows.first().unwrap().get::<_, bool>("finished"));
    })
    .await;
}

#[tokio::test]
async fn completion_single_unconfirmed_row() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let pending = migration(
            "20250104000000_unconfirmed",
            "CREATE TABLE pending (id int);",
        );
        insert_row(
            &database.pool,
            &LedgerRow {
                id: "row-unconfirmed",
                name: &pending.name,
                checksum: &pending.checksum,
                steps: 2,
                logs: Some(""),
                finished: false,
                rolled_back: false,
                started_at: FIXED_STARTED_AT,
            },
        )
        .await;
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        assert!(
            !recover_completed_migration(&coordinator, PUBLIC_SCHEMA, &pending, &DrawingJitter(0))
                .await
                .unwrap()
        );
        coordinator.finish().await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert!(!rows.first().unwrap().get::<_, bool>("finished"));
    })
    .await;
}

#[tokio::test]
async fn completion_updates_only_finished_at() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let pending = migration(
            "20250111000000_confirming",
            "CREATE TABLE confirming (id int);",
        );
        insert_confirmed(&database.pool, "row-confirming", &pending, false).await;
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        assert!(
            recover_completed_migration(&coordinator, PUBLIC_SCHEMA, &pending, &DrawingJitter(0))
                .await
                .unwrap()
        );
        coordinator.finish().await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].get::<_, bool>("finished"));
        assert!(!rows[0].get::<_, bool>("rolled_back"));
        assert_eq!(rows[0].get::<_, String>("id"), "row-confirming");
        assert_eq!(rows[0].get::<_, String>("checksum"), pending.checksum);
        assert_eq!(
            rows[0].get::<_, Option<String>>("logs").as_deref(),
            Some("")
        );
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 1_i32);
    })
    .await;
}
