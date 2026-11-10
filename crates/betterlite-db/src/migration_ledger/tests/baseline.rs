fn assert_drift(error: &BaselineError) {
    match error {
        BaselineError::SchemaMismatch { summary, .. } => {
            assert!(summary.contains("operator_notes"), "{summary}");
        }
        BaselineError::CustomHistory
        | BaselineError::Database(_)
        | BaselineError::Lock(_)
        | BaselineError::VerificationUnavailable
        | BaselineError::Verification(_) => panic!("expected schema drift, got {error:?}"),
    }
}

async fn refuse_baseline(database: &TestDatabase, catalog: &[Migration], schema_prisma: &[u8]) {
    let inputs = baseline_input(catalog, schema_prisma, None);
    let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
    let error = baseline_current_schema(&coordinator, &inputs, &DrawingJitter(0))
        .await
        .expect_err("a custom migration history is refused");
    assert!(matches!(error, BaselineError::CustomHistory), "{error:?}");
    drop(coordinator);
}

async fn accepted_baseline(live: &TestDatabase, expected: &SchemaContract) -> Vec<Migration> {
    apply_history(&live.pool, HistorySource::Compiled).await;
    let catalog = runtime_catalog().await;
    let schema_prisma = schema_prisma_bytes();
    let inputs = baseline_input(&catalog, &schema_prisma, Some(expected));
    let coordinator = acquired_coordinator(&live.pool, Duration::from_secs(30)).await;
    baseline_current_schema(&coordinator, &inputs, &DrawingJitter(0))
        .await
        .unwrap();
    coordinator.finish().await.unwrap();
    catalog
}

async fn assert_ledger_catalog(pool: &Pool) {
    let client = pool.get().await.unwrap();
    let columns = client
        .query(
            "SELECT column_name, data_type, character_maximum_length, is_nullable \
             FROM information_schema.columns WHERE table_schema = 'public' \
             AND table_name = '_prisma_migrations' ORDER BY ordinal_position",
            &[],
        )
        .await
        .unwrap();
    let described: Vec<(String, String, Option<i32>, String)> = columns
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect();
    let expected = [
        ("id", "character varying", Some(36_i32), "NO"),
        ("checksum", "character varying", Some(64_i32), "NO"),
        ("finished_at", "timestamp with time zone", None, "YES"),
        ("migration_name", "character varying", Some(255_i32), "NO"),
        ("logs", "text", None, "YES"),
        ("rolled_back_at", "timestamp with time zone", None, "YES"),
        ("started_at", "timestamp with time zone", None, "NO"),
        ("applied_steps_count", "integer", None, "NO"),
    ];
    assert_eq!(described.len(), expected.len());
    for (index, (name, data_type, length, nullable)) in expected.into_iter().enumerate() {
        let actual = &described[index];
        assert_eq!(actual.0, name);
        assert_eq!(actual.1, data_type);
        assert_eq!(actual.2, length);
        assert_eq!(actual.3, nullable);
    }
}

#[tokio::test]
async fn native_baseline_current_only() {
    database_pair(|live, reference| async move {
        apply_history(&reference.pool, HistorySource::Pinned).await;
        let expected = read_contract(&reference.pool).await;
        apply_history(&live.pool, HistorySource::Compiled).await;
        let catalog = runtime_catalog().await;
        let schema_prisma = schema_prisma_bytes();
        let inputs = baseline_input(&catalog, &schema_prisma, Some(&expected));
        let jitter = DrawingJitter(0);

        let operator = live.pool.get().await.unwrap();
        operator
            .batch_execute("CREATE TABLE operator_notes (id int)")
            .await
            .unwrap();
        drop(operator);
        let drifted = acquired_coordinator(&live.pool, Duration::from_secs(30)).await;
        let error = baseline_current_schema(&drifted, &inputs, &jitter)
            .await
            .expect_err("an operator relation is drift");
        assert_drift(&error);
        drop(drifted);
        assert!(!exists(&live.pool, "_prisma_migrations").await);

        let cleanup = live.pool.get().await.unwrap();
        cleanup
            .batch_execute("DROP TABLE operator_notes")
            .await
            .unwrap();
        drop(cleanup);
        let coordinator = acquired_coordinator(&live.pool, Duration::from_secs(30)).await;
        baseline_current_schema(&coordinator, &inputs, &jitter)
            .await
            .unwrap();
        coordinator.finish().await.unwrap();
        assert_eq!(ledger_rows(&live.pool).await.len(), catalog.len());
    })
    .await;
}

#[tokio::test]
async fn baseline_rows_zero_steps() {
    database(|database| async move {
        apply_history(&database.pool, HistorySource::Compiled).await;
        let expected = read_contract(&database.pool).await;
        let catalog = runtime_catalog().await;
        let schema_prisma = schema_prisma_bytes();
        let inputs = baseline_input(&catalog, &schema_prisma, Some(&expected));
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(30)).await;
        baseline_current_schema(&coordinator, &inputs, &DrawingJitter(0))
            .await
            .unwrap();
        coordinator.finish().await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert_eq!(rows.len(), catalog.len());
        assert!(
            rows.iter()
                .all(|row| row.get::<_, i32>("applied_steps_count") == 0_i32)
        );
        assert!(
            rows.iter()
                .all(|row| row.get::<_, String>("logs").is_empty())
        );
        assert!(
            rows.iter()
                .all(|row| row.get::<_, bool>("finished") && row.get::<_, bool>("equal_times"))
        );
        assert!(rows.iter().all(|row| !row.get::<_, bool>("rolled_back")));
        assert!(
            rows.iter()
                .all(|row| row.get::<_, String>("id").len() == 36)
        );
    })
    .await;
}

#[tokio::test]
async fn baseline_custom_history_refused() {
    database(|database| async move {
        let catalog = runtime_catalog().await;
        let schema_prisma = schema_prisma_bytes();
        let mut extended = catalog.clone();
        extended.push(migration("20250101000000_custom", "SELECT 1;"));
        refuse_baseline(&database, &extended, &schema_prisma).await;
        refuse_baseline(&database, &[], &schema_prisma).await;
        let mut tampered = catalog.clone();
        if let Some(first) = tampered.first_mut() {
            first.checksum = "0".repeat(64);
        }
        refuse_baseline(&database, &tampered, &schema_prisma).await;
        refuse_baseline(&database, &catalog, b"model User { id String @id }").await;
        assert!(!exists(&database.pool, "_prisma_migrations").await);
    })
    .await;
}

#[tokio::test]
async fn baseline_existing_ledger_noop() {
    database(|database| async move {
        create_ledger(&database.pool).await;
        insert_confirmed(
            &database.pool,
            "row-existing",
            &migration("20240101000000_old", "SELECT 0;"),
            true,
        )
        .await;
        let catalog = runtime_catalog().await;
        let schema_prisma = schema_prisma_bytes();
        let inputs = baseline_input(&catalog, &schema_prisma, None);
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(10)).await;
        baseline_current_schema(&coordinator, &inputs, &DrawingJitter(0))
            .await
            .unwrap();
        coordinator.finish().await.unwrap();
        let rows = ledger_rows(&database.pool).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get::<_, String>("migration_name"),
            "20240101000000_old"
        );
        assert_eq!(rows[0].get::<_, i32>("applied_steps_count"), 1_i32);
        assert!(rows[0].get::<_, bool>("finished"));
    })
    .await;
}

#[tokio::test]
async fn baseline_warns_backfills() {
    database(|database| async move {
        apply_history(&database.pool, HistorySource::Compiled).await;
        let expected = read_contract(&database.pool).await;
        let catalog = runtime_catalog().await;
        let schema_prisma = schema_prisma_bytes();
        let inputs = baseline_input(&catalog, &schema_prisma, Some(&expected));
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(30)).await;
        let (logs, subscriber) = capture_logs();
        let guard = set_default(subscriber);
        baseline_current_schema(&coordinator, &inputs, &DrawingJitter(0))
            .await
            .unwrap();
        coordinator.finish().await.unwrap();
        drop(guard);
        let captured = logs.contents();
        assert!(
            captured.contains("Legacy migration history was missing"),
            "{captured}"
        );
        assert!(
            captured.contains("backfills were not replayed or verified"),
            "{captured}"
        );
        assert!(captured.contains("adopted"), "{captured}");
        assert!(
            captured.contains(&format!("migrated={}", catalog.len())),
            "{captured}"
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_pinned_rows() {
    database_pair(|live, reference| async move {
        apply_history(&reference.pool, HistorySource::Pinned).await;
        let expected = read_contract(&reference.pool).await;
        let catalog = accepted_baseline(&live, &expected).await;
        let rows = ledger_rows(&live.pool).await;
        let pinned = pinned_catalog_map();
        assert_eq!(rows.len(), pinned.len());
        assert!(rows.iter().all(|row| {
            let name: String = row.get("migration_name");
            let checksum: String = row.get("checksum");
            pinned.get(&name) == Some(&checksum)
        }));
        assert_eq!(catalog.len(), rows.len());
        assert_ledger_catalog(&live.pool).await;
    })
    .await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    database(|database| async move {
        let expected = read_contract(&database.pool).await;
        let catalog = runtime_catalog().await;
        let schema_prisma = schema_prisma_bytes();
        let mut inputs = baseline_input(&catalog, &schema_prisma, Some(&expected));
        inputs.next_id = duplicated_id;
        let coordinator = acquired_coordinator(&database.pool, Duration::from_secs(30)).await;
        let error = baseline_current_schema(&coordinator, &inputs, &DrawingJitter(0))
            .await
            .expect_err("the duplicated identifier breaks the second insert");
        assert!(matches!(error, BaselineError::Database(_)), "{error:?}");
        drop(coordinator);
        assert!(!exists(&database.pool, "_prisma_migrations").await);
    })
    .await;
}

fn duplicated_id() -> Uuid {
    Uuid::nil()
}
