use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage(
        &store,
        &usage("row-1", "gpt-4o", Decimal::new(15, 1), minutes_ago(10)),
    )
    .await;
    let empty_transport = FakeTransport::default();
    let empty_logger = logger(
        settings(),
        clone_store(&store),
        empty_transport.clone(),
        None,
    );
    export_usage_data(
        &empty_logger,
        None,
        None,
        Some(minutes_ago(120).and_utc()),
        Some(minutes_ago(60).and_utc()),
    )
    .await
    .expect("an empty window");
    assert!(empty_transport.posts().is_empty());
    let failing_transport = FakeTransport::failing();
    let failing_logger = logger(
        settings(),
        clone_store(&store),
        failing_transport.clone(),
        None,
    );
    let error = export_usage_data(&failing_logger, None, None, None, None)
        .await
        .expect_err("a refused post");
    assert!(matches!(error, CloudZeroExportError::Delivery(_)));
    assert_eq!(failing_transport.posts().len(), 1);
    close(fixture, &store).await;
}
