#[tokio::test]
async fn postgres_pinned_rows() {
    Box::pin(with_databases(2, |mut databases| async move {
        let live = databases.remove(0);
        let pinned = databases.remove(0);
        reset_public(&live.pool).await;
        reset_public(&pinned.pool).await;
        apply_compiled(&live.pool).await;
        apply_pinned(&pinned.pool).await;
        let expected = read_contract(&pinned.pool, PUBLIC_SCHEMA).await;
        let actual = read_contract(&live.pool, PUBLIC_SCHEMA).await;
        assert!(expected.compare(&actual).differences.is_empty());
        let outcome = check_schema(
            &live.check(),
            true,
            Some(&expected),
            PUBLIC_SCHEMA,
            READ_TIMEOUT,
        )
        .await;
        assert_eq!(outcome, SchemaCheck::InSync);
        assert!(relation_present(&actual, "LiteLLM_Lens"));
        assert!(relation_present(&actual, "LiteLLM_LensRun"));
        assert!(relation_present(&actual, "LiteLLM_SpendLogs"));
        assert!(!relation_present(&actual, "LiteLLM_Engine"));
        assert!(!relation_present(&actual, "LiteLLM_EngineRun"));
    }))
    .await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    Box::pin(with_databases(1, |mut databases| async move {
        let database = databases.remove(0);
        let client = database.pool.get().await.unwrap();
        let raw = &**client;
        create_stuck_indexes(raw).await;
        let (logs, dispatch) = capture();
        let _guard = set_default(&dispatch);

        assert_lock_skips_repair(&database, raw, &logs).await;
        failed_repair(
            &database,
            "not a timeout",
            &logs,
            "could not check for invalid indexes",
        )
        .await;
        assert_still_invalid(raw, &database.schema).await;
        failed_repair(&database, "30s", &logs, "could not repair an invalid index").await;
        assert_eq!(
            index_valid(raw, &database.schema, "LiteLLM_aaa_repairable").await,
            Some(true)
        );
        assert_eq!(
            index_valid(raw, &database.schema, "LiteLLM_zzz_stuck").await,
            Some(false)
        );
        deduplicate(raw, "LiteLLM_Stuck").await;
        assert_complete_repair(&database, raw).await;
    }))
    .await;
}

#[tokio::test]
async fn native_native_pg_catalog_comparison_replaces_prisma_migrate_diff_and_remains() {
    Box::pin(with_schema(|database| async move {
        let client = database.pool.get().await.unwrap();
        client
            .batch_execute("CREATE TABLE \"LiteLLM_Native\" (id text PRIMARY KEY)")
            .await
            .unwrap();
        let expected = read_contract(&database.pool, &database.schema).await;
        let (logs, dispatch) = capture();
        let _guard = set_default(&dispatch);
        let in_sync = schema_check(&database, &expected, true).await;
        assert_eq!(in_sync, SchemaCheck::InSync);
        client
            .batch_execute("CREATE INDEX \"extra_idx\" ON \"LiteLLM_Native\" (id)")
            .await
            .unwrap();
        let outcome = schema_check(&database, &expected, true).await;
        let SchemaCheck::Differing(comparison) = outcome else {
            panic!("a catalog index added after the expected contract must be drift");
        };
        assert!(changed(
            &comparison,
            ObjectKind::Index,
            "LiteLLM_Native",
            "extra_idx",
            Presence::Extra
        ));
        assert!(logs.joined().contains("out of sync"));
    }))
    .await;
}
