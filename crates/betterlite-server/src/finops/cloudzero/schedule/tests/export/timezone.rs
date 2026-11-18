use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    let mut offset = usage("row-tz", "gpt-4o", Decimal::ONE, minutes_ago(10));
    offset.date = "2026-02-01T04:30:00+00:00";
    insert_usage(&store, &offset).await;
    let utc_transport = FakeTransport::default();
    let utc_logger = logger(settings(), clone_store(&store), utc_transport.clone(), None);
    export_usage_data(&utc_logger, None, None, None, None)
        .await
        .expect("a UTC export");
    assert_eq!(utc_transport.posts()[0].month, "2026-02");
    let tokyo_transport = FakeTransport::default();
    let tokyo = settings_from(&[("CLOUDZERO_TIMEZONE", "Asia/Tokyo")]);
    let tokyo_logger = logger(tokyo, clone_store(&store), tokyo_transport.clone(), None);
    export_usage_data(&tokyo_logger, None, None, None, None)
        .await
        .expect("a Tokyo export");
    assert_eq!(tokyo_transport.posts()[0].month, "2026-01");
    close(fixture, &store).await;
}
