#[rstest]
#[case::empty("")]
#[case::blank("  \n ")]
#[case::line_comment("-- just a comment\n")]
#[case::block_comment("/* block\n comment */")]
#[case::select_one("SELECT 1;")]
#[case::lowercase("select 1 ; select 1")]
#[case::padded("  SELECT   1  ;\n")]
#[case::commented_out_ddl("-- CREATE INDEX x ON t(a);\nSELECT 1;")]
#[case::block_before_select("/* CREATE INDEX x ON t(a); */ SELECT 1;")]
fn inert_comments_select_one(#[case] script: &str) {
    assert!(is_inert_migration(script));
}

#[rstest]
#[case::select_two("SELECT 2;")]
#[case::ddl("CREATE INDEX x ON t(a);")]
#[case::mixed("SELECT 1; CREATE INDEX x ON t(a);")]
#[case::trailing_ddl("SELECT 1; -- done\nCREATE INDEX x ON t(a);")]
fn inert_rejects_changes(#[case] script: &str) {
    assert!(!is_inert_migration(script));
}

#[tokio::test]
async fn noninert_not_rolled_back() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let ddl = migration(
            "20250105000000_noninert",
            "CREATE INDEX created ON installed (id);",
        );
        insert_failed(&database.pool, "row-noninert", &ddl, FIXED_STARTED_AT).await;
        let holder = connection(&database.pool).await;
        holder.batch_execute("BEGIN").await.unwrap();
        assert!(xact_probe(&holder, PRISMA_LOCK_KEY).await);
        let coordinator = acquired_coordinator(&database.pool, Duration::from_millis(300)).await;
        let rolled_back =
            rollback_inert_migration(&coordinator, PUBLIC_SCHEMA, &ddl, &DrawingJitter(0))
                .await
                .unwrap();
        assert!(!rolled_back);
        coordinator.finish().await.unwrap();
        holder.batch_execute("COMMIT").await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].get::<_, bool>("rolled_back"));
    })
    .await;
}

#[tokio::test]
async fn failed_inert_recovery() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let inert = migration("20250106000000_inert", "-- retired\nSELECT 1;\n");
        insert_failed(&database.pool, "row-inert", &inert, FIXED_STARTED_AT).await;
        let jitter = DrawingJitter(0);
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        assert!(
            rollback_inert_migration(&coordinator, PUBLIC_SCHEMA, &inert, &jitter)
                .await
                .unwrap()
        );
        coordinator.finish().await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].get::<_, bool>("rolled_back"));
        assert!(!rows[0].get::<_, bool>("finished"));
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 1_i32);
        assert_eq!(
            rows[0].get::<_, Option<String>>("logs").as_deref(),
            Some("boom")
        );
        assert_eq!(rows[0].get::<_, String>("checksum"), inert.checksum);
    })
    .await;
}

#[tokio::test]
async fn inert_already_rolled_back() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let inert = migration("20250107000000_inert", "SELECT 1;");
        insert_failed(&database.pool, "row-inert", &inert, FIXED_STARTED_AT).await;
        let jitter = DrawingJitter(0);
        let first = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        assert!(
            rollback_inert_migration(&first, PUBLIC_SCHEMA, &inert, &jitter)
                .await
                .unwrap()
        );
        first.finish().await.unwrap();
        let second = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        assert!(
            !rollback_inert_migration(&second, PUBLIC_SCHEMA, &inert, &jitter)
                .await
                .unwrap()
        );
        second.finish().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn inert_invalid_utf8() {
    database(|database| async move {
        let invalid = Migration {
            name: "20250108000000_invalid".to_owned(),
            checksum: checksum_of("SELECT 1;"),
            sql: vec![0xff, 0xfe, 0x00],
        };
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        let error =
            rollback_inert_migration(&coordinator, PUBLIC_SCHEMA, &invalid, &DrawingJitter(0))
                .await
                .expect_err("the script is not UTF-8");
        assert!(matches!(error, InertError::InvalidSql { .. }), "{error:?}");
        coordinator.finish().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn inert_guard_rowcount() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        let inert = migration("20250109000000_guarded", "SELECT 1;");
        insert_failed(&database.pool, "row-guarded", &inert, FIXED_STARTED_AT).await;
        let client = database.pool.get().await.unwrap();
        client
            .batch_execute(
                "CREATE RULE suppress_rollback AS ON UPDATE TO _prisma_migrations \
                 DO INSTEAD NOTHING",
            )
            .await
            .unwrap();
        drop(client);
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        let error =
            rollback_inert_migration(&coordinator, PUBLIC_SCHEMA, &inert, &DrawingJitter(0))
                .await
                .expect_err("the guard cannot update the row");
        assert!(matches!(error, InertError::RollbackMismatch), "{error:?}");
        coordinator.finish().await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].get::<_, bool>("rolled_back"));
    })
    .await;
}
