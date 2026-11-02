#[tokio::test]
async fn postgres_catalog() {
    Box::pin(harness(|[native, pinned]| async move {
        apply_migrations_all(&native).await;
        apply_pinned_migrations_all(&pinned).await;

        let native_snapshot = catalog_snapshot(&native).await;
        let pinned_snapshot = catalog_snapshot(&pinned).await;
        assert_eq!(native_snapshot, pinned_snapshot);
        assert!(!native_snapshot.is_empty());
    }))
    .await;
}
