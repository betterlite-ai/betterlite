#[tokio::test]
async fn lens_rename_guard() {
    Box::pin(with_schema(|database| async move {
        let client = database.pool.get().await.unwrap();
        let check = database.check();
        raise_if_lens_rename_pending(&check).await.unwrap();
        client
            .batch_execute("CREATE TABLE \"LiteLLM_Engine\" (id text PRIMARY KEY)")
            .await
            .unwrap();
        assert!(matches!(
            raise_if_lens_rename_pending(&check).await,
            Err(LensGuardError::LegacyLensTables)
        ));
        client
            .batch_execute(
                "DROP TABLE \"LiteLLM_Engine\"; \
                 CREATE TABLE \"LiteLLM_Lens\" (id text PRIMARY KEY)",
            )
            .await
            .unwrap();
        raise_if_lens_rename_pending(&check).await.unwrap();
    }))
    .await;
}

#[tokio::test]
async fn lens_guard_reports_unavailable_database() {
    let absent = MigrationDriftCheck {
        database_url: None,
        direct_url: None,
    };
    raise_if_lens_rename_pending(&absent).await.unwrap();
    let malformed = MigrationDriftCheck {
        database_url: Some(SecretString::from("postgresql://[::1/db")),
        direct_url: None,
    };
    assert!(matches!(
        raise_if_lens_rename_pending(&malformed).await,
        Err(LensGuardError::Schema { .. })
    ));
    let unreachable = MigrationDriftCheck {
        database_url: Some(SecretString::from(
            "postgresql://postgres:postgres@127.0.0.1:1/postgres",
        )),
        direct_url: None,
    };
    assert!(matches!(
        raise_if_lens_rename_pending(&unreachable).await,
        Err(LensGuardError::Connect(_))
    ));
}

#[tokio::test]
async fn partition_guard_detects_partitioned_spend_logs() {
    Box::pin(with_schema(|database| async move {
        let client = database.pool.get().await.unwrap();
        let check = database.check();
        assert!(!spend_logs_is_partitioned(&check).await);
        client
            .batch_execute(
                "CREATE TABLE \"LiteLLM_SpendLogs\" \
                 (\"request_id\" text NOT NULL, \"startTime\" timestamptz NOT NULL)",
            )
            .await
            .unwrap();
        assert!(!spend_logs_is_partitioned(&check).await);
        client
            .batch_execute(
                "DROP TABLE \"LiteLLM_SpendLogs\"; \
                 CREATE TABLE \"LiteLLM_SpendLogs\" \
                 (\"request_id\" text NOT NULL, \"startTime\" timestamptz NOT NULL) \
                 PARTITION BY RANGE (\"startTime\")",
            )
            .await
            .unwrap();
        assert!(spend_logs_is_partitioned(&check).await);
    }))
    .await;
}

#[tokio::test]
async fn partition_guard_unavailable_is_false() {
    let absent = MigrationDriftCheck {
        database_url: None,
        direct_url: None,
    };
    assert!(!spend_logs_is_partitioned(&absent).await);
    let malformed = MigrationDriftCheck {
        database_url: Some(SecretString::from("postgresql://[::1/db")),
        direct_url: None,
    };
    assert!(!spend_logs_is_partitioned(&malformed).await);
    let unreachable = MigrationDriftCheck {
        database_url: Some(SecretString::from(
            "postgresql://postgres:postgres@127.0.0.1:1/postgres",
        )),
        direct_url: None,
    };
    assert!(!spend_logs_is_partitioned(&unreachable).await);
}
