use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage(
        &store,
        &usage("row-1", "gpt-4o", Decimal::ONE, minutes_ago(10)),
    )
    .await;
    let unavailable_transport = FakeTransport::default();
    let unavailable = Arc::new(FakeLease::new(false, Some(false)));
    let unavailable_logger = logger(
        settings(),
        clone_store(&store),
        unavailable_transport.clone(),
        leased(&unavailable),
    );
    initialize_export_job(&unavailable_logger)
        .await
        .expect("a direct export");
    assert_eq!(unavailable_transport.posts().len(), 1);
    assert!(unavailable.events().is_empty());
    let unlocked_transport = FakeTransport::default();
    let unlocked_logger = logger(
        settings(),
        clone_store(&store),
        unlocked_transport.clone(),
        None,
    );
    initialize_export_job(&unlocked_logger)
        .await
        .expect("an unlocked export");
    assert_eq!(unlocked_transport.posts().len(), 1);
    close(fixture, &store).await;
}
