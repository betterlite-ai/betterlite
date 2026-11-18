use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let (fixture, store) = database().await;
    let mut zero = usage("row-zero", "gpt-4o", Decimal::new(15, 1), minutes_ago(10));
    zero.successful_requests = 0;
    insert_usage(&store, &zero).await;
    let transport = FakeTransport::default();
    let assembled = logger(settings(), clone_store(&store), transport.clone(), None);
    export_usage_data(&assembled, None, None, None, None)
        .await
        .expect("a skipped export");
    assert!(transport.posts().is_empty());
    close(fixture, &store).await;
}
