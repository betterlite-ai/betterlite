use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage(
        &store,
        &usage("row-new", "gpt-4o", Decimal::new(15, 1), minutes_ago(10)),
    )
    .await;
    insert_usage(
        &store,
        &usage(
            "row-mid",
            "gpt-4o-mini",
            Decimal::new(25, 1),
            minutes_ago(30),
        ),
    )
    .await;
    insert_usage(
        &store,
        &usage("row-old", "gpt-4", Decimal::new(35, 1), minutes_ago(200)),
    )
    .await;
    let transport = FakeTransport::default();
    let assembled = logger(settings(), clone_store(&store), transport.clone(), None);
    export_usage_data(
        &assembled,
        None,
        None,
        Some(minutes_ago(60).and_utc()),
        Some(now()),
    )
    .await
    .expect("a windowed export");
    assert_eq!(transport.posts()[0].data.len(), 2);
    close(fixture, &store).await;
}
