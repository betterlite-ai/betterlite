use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    let assembled = logger(
        settings(),
        clone_store(&store),
        FakeTransport::default(),
        None,
    );
    let outcome = dry_run_export_usage_data(&assembled, None)
        .await
        .expect("an empty dry run");
    assert!(outcome.usage_data.is_empty());
    assert!(outcome.cbf_data.is_empty());
    assert_eq!(outcome.summary.total_records, 0);
    assert_eq!(outcome.summary.total_cost, Decimal::ZERO);
    assert_eq!(outcome.summary.total_tokens, 0);
    assert_eq!(outcome.summary.unique_accounts, 0);
    assert_eq!(outcome.summary.unique_services, 0);
    close(fixture, &store).await;
}
