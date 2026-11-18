use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage(
        &store,
        &usage("row-1", "gpt-4o", Decimal::ONE, minutes_ago(10)),
    )
    .await;
    let transport = FakeTransport::default();
    let lease = Arc::new(FakeLease::new(true, Some(true)));
    let assembled = logger(
        settings(),
        clone_store(&store),
        transport.clone(),
        leased(&lease),
    );
    initialize_export_job(&assembled)
        .await
        .expect("a locked export");
    assert_eq!(transport.posts().len(), 1);
    assert_eq!(lease.events(), lease_events(EXPORT_JOB_NAME));
    close(fixture, &store).await;
}
