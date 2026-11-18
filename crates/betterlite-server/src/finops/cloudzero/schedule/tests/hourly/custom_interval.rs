use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage_specs(
        &store,
        &[
            usage("row-61", "model-a", Decimal::ONE, minutes_ago(61)),
            usage("row-60", "model-b", Decimal::ONE, minutes_ago(60)),
            usage("row-59", "model-c", Decimal::ONE, minutes_ago(59)),
            usage("row-31", "model-d", Decimal::ONE, minutes_ago(31)),
        ],
    )
    .await;
    let transport = FakeTransport::default();
    let settings = settings_from(&[("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "30")]);
    let assembled = logger(settings, clone_store(&store), transport.clone(), None);
    hourly_usage_data_export(&assembled)
        .await
        .expect("the custom export");
    assert_eq!(transport.posts()[0].data.len(), 3);
    close(fixture, &store).await;
}
