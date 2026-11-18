use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_generated_usage(&store, minutes_ago(10), 60).await;
    let assembled = logger(
        settings(),
        clone_store(&store),
        FakeTransport::default(),
        None,
    );
    let full = dry_run_export_usage_data(&assembled, None)
        .await
        .expect("a full dry run");
    assert_eq!(full.usage_data.len(), 50);
    assert_eq!(full.cbf_data.len(), 60);
    assert_eq!(full.summary.total_records, 60);
    assert_eq!(full.summary.total_cost, Decimal::new(90, 0));
    assert_eq!(full.summary.total_tokens, 180);
    assert_eq!(full.summary.unique_accounts, 1);
    assert_eq!(full.summary.unique_services, 1);
    let limited = dry_run_export_usage_data(&assembled, Some(3))
        .await
        .expect("a limited dry run");
    assert_eq!(limited.usage_data.len(), 3);
    assert_eq!(limited.cbf_data.len(), 3);
    assert_eq!(limited.summary.total_records, 3);
    assert_eq!(limited.summary.total_cost, Decimal::new(45, 1));
    assert_eq!(limited.summary.total_tokens, 9);
    close(fixture, &store).await;
}
