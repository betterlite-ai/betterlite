#[tokio::test]
async fn postgres_catalog() {
    Box::pin(harness(|[native, pinned]| async move {
        batch_execute(&native, &migration_sql_all()).await;
        batch_execute(&pinned, &pinned_migration_sql_all()).await;

        let native_snapshot = catalog_snapshot(&native).await;
        let pinned_snapshot = catalog_snapshot(&pinned).await;
        assert_eq!(native_snapshot, pinned_snapshot);
        assert!(!native_snapshot.is_empty());
    }))
    .await;
}
