use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage_specs(
        &store,
        &[
            usage("row-10", "model-a", Decimal::ONE, minutes_ago(10)),
            usage("row-11", "model-b", Decimal::ONE, minutes_ago(11)),
            usage("row-12", "model-c", Decimal::ONE, minutes_ago(12)),
        ],
    )
    .await;
    let transport = FakeTransport::default();
    let settings = settings_from(&[("CLOUDZERO_MAX_FETCHED_DATA_RECORDS", "1")]);
    let assembled = logger(settings, clone_store(&store), transport.clone(), None);
    hourly_usage_data_export(&assembled)
        .await
        .expect("the capped export");
    assert_eq!(transport.posts()[0].data.len(), 1);
    close(fixture, &store).await;
}
