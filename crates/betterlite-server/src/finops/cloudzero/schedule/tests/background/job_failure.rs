use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage(
        &store,
        &usage("row-1", "gpt-4o", Decimal::ONE, minutes_ago(10)),
    )
    .await;
    let transport = FakeTransport::failing();
    let lease = Arc::new(FakeLease::new(true, Some(true)));
    let assembled = Arc::new(logger(
        settings(),
        clone_store(&store),
        transport.clone(),
        Some(leased(&lease)),
    ));
    let mut scheduler = FakeScheduler::default();
    init_cloudzero_background_job(&mut scheduler, &[assembled]);
    scheduler.jobs[0].2.run().await;
    assert_eq!(transport.posts().len(), 1);
    assert_eq!(lease.events(), lease_events(EXPORT_JOB_NAME));
    close(fixture, &store).await;
}
