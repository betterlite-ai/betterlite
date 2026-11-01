async fn catalog_snapshot(client: &Client) -> Vec<String> {
    let mut snapshot = string_rows(
        client,
        "SELECT table_name || ':' || column_name || ':' || data_type || ':' || is_nullable \
         || ':' || coalesce(column_default, '') FROM information_schema.columns \
         WHERE table_schema = 'public' ORDER BY table_name, ordinal_position",
        &[],
    )
    .await;
    let mut indexes = string_rows(
        client,
        "SELECT indexname || ':' || indexdef FROM pg_indexes \
         WHERE schemaname = 'public' ORDER BY indexname",
        &[],
    )
    .await;
    snapshot.append(&mut indexes);
    snapshot
}

#[tokio::test]
async fn postgres_pinned_rows() {
    Box::pin(harness(|[native, pinned]| async move {
        let sql = migration_sql_all();
        batch_execute(&native, &sql).await;
        batch_execute(&pinned, &sql).await;

        assert_rule(
            &native,
            "LiteLLM_EndUserTable_object_permission_id_fkey",
            'c',
            'n',
        )
        .await;
        assert_rule(
            &pinned,
            "LiteLLM_EndUserTable_object_permission_id_fkey",
            'c',
            'n',
        )
        .await;

        let native_snapshot = catalog_snapshot(&native).await;
        let pinned_snapshot = catalog_snapshot(&pinned).await;
        assert_eq!(native_snapshot, pinned_snapshot);
        assert!(!native_snapshot.is_empty());
    }))
    .await;
}
