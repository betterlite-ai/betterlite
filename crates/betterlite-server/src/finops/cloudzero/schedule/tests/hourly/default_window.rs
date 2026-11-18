use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    insert_usage_specs(
        &store,
        &[
            usage("row-121", "model-a", Decimal::ONE, minutes_ago(121)),
            usage("row-120", "model-b", Decimal::ONE, minutes_ago(120)),
            usage("row-119", "model-c", Decimal::ONE, minutes_ago(119)),
        ],
    )
    .await;
    let transport = FakeTransport::default();
    let assembled = logger(settings(), clone_store(&store), transport.clone(), None);
    hourly_usage_data_export(&assembled)
        .await
        .expect("the hourly export");
    let posts = transport.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].operation, "replace_hourly");
    assert_eq!(posts[0].data.len(), 2);
    close(fixture, &store).await;
}
