use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage(
        &store,
        &usage("row-1", "gpt-4o", Decimal::new(15, 1), minutes_ago(10)),
    )
    .await;
    let default_transport = FakeTransport::default();
    let default_logger = logger(
        settings(),
        clone_store(&store),
        default_transport.clone(),
        None,
    );
    export_usage_data(&default_logger, None, None, None, None)
        .await
        .expect("a default export");
    let posts = default_transport.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].operation, "replace_hourly");
    assert_eq!(posts[0].month, "2026-01");
    assert_eq!(posts[0].data.len(), 1);
    assert_eq!(
        posts[0].data[0].get("cost/cost").map(String::as_str),
        Some("1.5")
    );
    let sum_transport = FakeTransport::default();
    let sum_logger = logger(settings(), clone_store(&store), sum_transport.clone(), None);
    export_usage_data(&sum_logger, None, Some(ExportOperation::Sum), None, None)
        .await
        .expect("a sum export");
    assert_eq!(sum_transport.posts()[0].operation, "sum");
    close(fixture, &store).await;
}
