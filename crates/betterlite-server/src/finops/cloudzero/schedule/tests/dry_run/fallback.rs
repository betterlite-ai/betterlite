use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    let mut first = usage("row-low-1", "model-a", Decimal::new(25, 1), minutes_ago(5));
    first.successful_requests = 0;
    insert_usage(&store, &first).await;
    let mut second = usage("row-low-2", "model-b", Decimal::new(35, 1), minutes_ago(5));
    second.successful_requests = 0;
    insert_usage(&store, &second).await;
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let unconfigured = CloudZeroSettings::resolve(None, None, None, &empty).expect("settings");
    let assembled = logger(
        unconfigured,
        clone_store(&store),
        FakeTransport::default(),
        None,
    );
    let outcome = dry_run_export_usage_data(&assembled, None)
        .await
        .expect("a fallback dry run");
    assert_eq!(outcome.usage_data.len(), 2);
    assert!(outcome.cbf_data.is_empty());
    assert_eq!(outcome.summary.total_records, 2);
    assert_eq!(outcome.summary.total_cost, Decimal::new(6, 0));
    assert_eq!(outcome.summary.total_tokens, 6);
    assert_eq!(outcome.summary.unique_accounts, 0);
    assert_eq!(outcome.summary.unique_services, 0);
    close(fixture, &store).await;
}
